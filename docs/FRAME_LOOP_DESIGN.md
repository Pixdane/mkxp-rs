# mkxp-rs 帧循环设计：winit 为主体

本文档定义 mkxp-rs 中 winit 事件循环和 Ruby 游戏逻辑之间的帧调度协议。

## 问题

mkxp-z 的帧循环以 Ruby 为外层驱动：

```
Ruby 线程（游戏线程）
  loop do
    Graphics.update()    ← Ruby 主动调用，C++ 渲染一帧
    Input.update()       ← 刷新按键状态
    $scene.update        ← 纯 Ruby 游戏逻辑
  end

主线程（事件线程）
  SDL_WaitEvent() 循环   ← 处理窗口事件、键盘鼠标
```

两个线程共享 OpenGL 上下文（通过 `SDL_GL_MakeCurrent` 切换），
共享全局输入数组（`keyStates`），通过 `UnidirMessage` 和 `AtomicFlag` 通信。

在 mkxp-rs 中，**winit 必须是事件循环的唯一主体**。
`EventLoop::run()` 接管整个线程，不能有另一个 `loop` 和它竞争。
但 Ruby（magnus/MRI）需要在某个线程里执行 `loop { Graphics.update }`。

这就产生了控制权归属的冲突：谁当外层循环？

## 方案：winit 主导，Ruby 作为帧回调

```
winit 线程（主线程，唯一的事件循环）
  EventLoop::run()
    ├─ Resized / Keyboard / Mouse 事件
    │    → 更新输入状态（winit 直接捕获，不进 Ruby）
    │
    └─ AboutToWait（空闲 = 帧边界）
         │
         ├─ 1. 唤醒 Ruby 线程："跑一帧逻辑"
         │      Ruby 线程执行：
         │        $scene.update
         │        Input.update（读取已缓存的输入状态）
         │        Graphics.update → 阻塞，说"我准备好了"
         │
         ├─ 2. 执行实际渲染
         │      GraphicsState::update()
         │        scene_graph.composite()
         │        后处理
         │        present
         │
         └─ 3. 通知 Ruby 线程："渲染完成，继续"
                Ruby 线程从 Graphics.update 返回
                继续下一帧的 $scene.update ...
```

Ruby 不再是驾驶员。它是乘客——winit 每帧叫它一次，它跑完自己的逻辑后交出控制权。

## 线程模型

```
┌─────────────────────────────────┐
│  winit 线程（主线程）            │
│                                 │
│  GameWindow                     │
│  GraphicsState（device/queue）  │
│  输入状态（只写）                │
│                                 │
│  EventLoop::run() {             │
│    AboutToWait => {             │
│      ruby_resume();   // ①     │
│      ruby_wait();      // ②     │
│      graphics.update(); // ③   │
│      ruby_signal();     // ④    │
│    }                            │
│  }                              │
└────────────┬────────────────────┘
             │  channel / condvar
┌────────────┴────────────────────┐
│  Ruby 线程（magnus/MRI）        │
│                                 │
│  loop do                        │
│    Input.update   ← 读输入状态  │
│    $scene.update  ← 游戏逻辑    │
│    Graphics.update              │
│      → ① 通知 winit            │
│      → ② 阻塞等待              │
│      → ④ 被唤醒，返回           │
│  end                            │
└─────────────────────────────────┘
```

### 为什么 Ruby 必须在独立线程

magnus 嵌入的 MRI Ruby 解释器有 GVL（Global VM Lock）。Ruby 代码
必须在持有 GVL 的线程中执行。winit 的事件循环也要求独占当前线程。
两者不能共用一个线程——必须分开。

这和 mkxp-z 的结构完全一致：winit 线程对应 EventThread，
Ruby 线程对应 RGSS 线程。

## 帧生命周期（一帧的完整过程）

```
时间轴 →

winit 线程                    Ruby 线程
──────────                    ──────────
AboutToWait 到达
│
├─→ signal_frame_start() ──→ 被唤醒
│                             │
│                             ├─ Input.update()
│                             │   读取 winit 线程缓存的按键状态
│                             │
│                             ├─ $scene.update
│                             │   游戏逻辑：移动精灵、检查碰撞...
│                             │
│                             ├─ 可能创建/销毁 Sprite/Bitmap
│                             │   这些操作影响 SceneGraph
│                             │
│                             └─ Graphics.update()
│                                  → frame_ready.signal()  ──→ 收到信号
│                                  → frame_done.wait()          │
│                                     （阻塞）                  │
│                                                              ├─ scene_graph.composite()
│                                                              │  遍历场景树，元素画自己
│                                                              │
│                                                              ├─ 后处理
│                                                              │  色调/亮度/颜色叠加
│                                                              │
│                                                              ├─ queue.submit()
│                                                              ├─ frame.present()
│                                                              │
│                                                              └─ frame_done.signal() ──→ 被唤醒
│                                                                    Graphics.update 返回
│                                                                    回到 loop 顶部
│                                                                    下一帧的 Input.update...
│
├─→ 回到 EventLoop，等下一个 AboutToWait
│   （期间处理窗口事件、输入事件）
│
└─→ 下一个 AboutToWait ...
```

## 同步原语设计

用两个同步点控制帧的交替执行：

```rust
use std::sync::{Arc, Condvar, Mutex};

/// winit 和 Ruby 线程之间的帧同步。
struct FrameSync {
    /// Ruby 线程是否已准备好（调用了 Graphics.update）。
    /// true = Ruby 可以渲染了，winit 应该执行 GraphicsState::update
    ready: Mutex<bool>,
    /// 条件变量：Ruby 等 winit 渲染完成，winit 等 Ruby 准备好。
    cv: Condvar,
}
```

两个关键操作：

```rust
impl FrameSync {
    /// Ruby 线程调用：通知 winit "我准备好了"，然后阻塞等渲染完成。
    fn ruby_frame_ready_and_wait(&self) {
        let mut ready = self.ready.lock().unwrap();
        *ready = true;
        self.cv.notify_one();            // 唤醒 winit
        while *ready {
            ready = self.cv.wait(ready).unwrap();  // 等 winit 渲染完
        }
    }

    /// winit 线程调用：等 Ruby 准备好，渲染，然后唤醒 Ruby。
    fn winit_render_and_signal(&self) -> bool {
        let mut ready = self.ready.lock().unwrap();
        if !*ready {
            return false;  // Ruby 还没准备好，跳过这帧
        }
        // ready == true，执行渲染...
        *ready = false;
        self.cv.notify_one();            // 唤醒 Ruby
        true
    }
}
```

只用了一个 Mutex + Condvar 和一个 bool。Ruby 侧和 winit 侧轮流翻转这个 bool。

## Graphics.update 在绑定层的实现

Ruby 调用的 `Graphics.update` 不是直接渲染，而是一个同步点：

```rust
// magnus 绑定
fn graphics_update(frame_sync: Arc<FrameSync>) -> magnus::Value {
    // 1. 通知 winit："我准备好了"
    frame_sync.ruby_frame_ready_and_wait();

    // 2. 检查是否需要退出
    if shutdown_requested() {
        // 抛出让 loop 退出的异常
    }

    // 3. 返回，Ruby 继续下一帧逻辑
    Qnil.into()
}
```

对游戏脚本来说，`Graphics.update` 的行为和 mkxp-z 完全一样——调用后画面更新了一帧。它不知道内部做了什么。

## 输入处理

winit 线程直接捕获所有输入事件（键盘、鼠标、手柄）、写入共享状态。
Ruby 线程的 `Input.update()` 读取这个共享状态，不通过事件队列中转。

这和 mkxp-z 的模型一致——`EventThread::keyStates` 全局数组
被 RGSS 线程直接读。区别是我们不需要 mutex（输入数据是简单的
原子数组，两个线程一写一读不需要锁）。

```rust
/// winit 线程写，Ruby 线程读。
struct InputState {
    /// 键盘按键状态。下标是 scancode。
    keys: [AtomicBool; KEY_COUNT],
    /// 鼠标位置。
    mouse_x: AtomicI32,
    mouse_y: AtomicI32,
    /// 鼠标按键。
    mouse_buttons: [AtomicBool; 8],
}
```

## 退出和重置

退出和 F12 重置通过共享信号，不经过事件队列：

```rust
/// 跨线程控制信号。
struct ControlSignals {
    /// winit 设 true → Ruby 线程在下次 Graphics.update 时抛出退出异常。
    shutdown: AtomicBool,
    /// F12 按下时设 true → Ruby 线程在下次 Graphics.update 时抛出 Reset 异常。
    reset: AtomicBool,
}
```

退出流程：

```
winit 收到 Close 事件
  → signals.shutdown = true
  → 等 Ruby 线程退出

Ruby 线程在 Graphics.update 中检查
  → signals.shutdown == true
  → 抛出异常，跳出 loop
  → Ruby 线程结束
```

F12 重置流程：

```
winit 收到 F12 按键
  → signals.reset = true

Ruby 线程在 Graphics.update 中检查
  → signals.reset == true
  → 抛出 Reset 异常
  → rgss_main 捕获，清除场景，重新调用 block
  → 游戏从头开始
```

## 窗口缩放事件

缩放发生在 winit 线程。如果 Ruby 正在执行游戏逻辑（还没到
`Graphics.update`），缩放事件的处理顺序是：

```
AboutToWait 之前：
  winit 收到 Resized(w, h)
    → 记录新尺寸到原子变量
    → 本轮不触发 Ruby 重绘（Ruby 还在跑）

AboutToWait：
  1. Ruby 跑到 Graphics.update → frame_ready = true
  2. winit 发现 resize_dirty = true
     → graphics.on_resize(new_w, new_h)
     → surface 重新配置
  3. graphics.update() 用新尺寸渲染
  4. Ruby 被唤醒，下一帧用新尺寸继续
```

## 与 mkxp-z 的对比

| | mkxp-z | mkxp-rs（winit 为主体） |
|---|---|---|
| 外层循环 | Ruby `loop` + SDL 事件循环（双线程） | winit EventLoop（主）+ Ruby 线程（辅） |
| 帧驱动者 | Ruby 主动调 `Graphics.update` | winit `AboutToWait` 触发一切 |
| Ruby `Graphics.update` | 直接执行渲染 | 同步点：通知 winit + 等待渲染完成 |
| 窗口事件 | EventThread 处理，通过 SDL 用户事件中转 | winit 线程直接处理 |
| 输入 | 全局静态数组，RGSS 线程直接读 | 原子数组，winit 写 Ruby 读 |
| 线程同步 | UnidirMessage + AtomicFlag + SyncPoint | 一个 Mutex + Condvar + 几个 AtomicBool |
| OpenGL 上下文切换 | `SDL_GL_MakeCurrent` 在两个线程间切换 | 不需要切换——只有 winit 线程持有 wgpu 上下文 |

## 游戏脚本不用改

关键的兼容性保证：游戏脚本里的 `loop { Graphics.update; ... }` 写法和
mkxp-z 完全一样。`Graphics.update` 的行为从脚本角度看没有变化——
调用后画面更新了一帧。内部的线程协同对 Ruby 脚本完全透明。

## 极端情况

**Ruby 的一帧耗时超过一帧的时间**（比如加载大场景）：
winit 的 `AboutToWait` 到达时 `frame_ready` 还是 `false`。
`winit_render_and_signal()` 返回 false，winit 跳过渲染，
继续处理窗口事件。等 Ruby 准备好后再渲染。表现为"卡顿但窗口不冻结"。

**Ruby 出错**：异常的捕获和 mkxp-z 一样——Ruby 侧 `rgss_main` 的
`rescue` 捕获所有异常，弹错误对话框。Reset 异常被特殊处理。

**窗口最小化**：`wgpu::Surface::get_current_texture()` 返回
`SurfaceError::Lost` 或 `Timeout`。winit 跳过渲染，等恢复。
Ruby 线程不受影响，继续跑逻辑（但 `Graphics.update` 会立刻返回，
不等待实际渲染）。
---

## 附录：双缓冲优化（搁置，等实测性能有问题再启用）

当前 v1 方案是严格交替的：Ruby 跑逻辑时 winit 闲置，winit 渲染时 Ruby 阻塞。
对于 RPG Maker 典型的 40fps 目标，每帧 25ms 预算中 Ruby 游戏逻辑通常只占几毫秒，
串行交替的浪费可以接受。

如果实测发现性能瓶颈（例如大地图场景 Ruby 逻辑耗时接近帧预算），可启用场景图
双缓冲：

- 场景图维护 front/back 两份状态
- Ruby 始终修改 back，winit 始终读取 front
- `Graphics.update()` 只交换缓冲区 + 发信号，不阻塞
- Ruby 立刻进入下一帧逻辑，与 winit 的渲染并行

代价是一帧画面延迟（~25ms），对回合制/剧情驱动的 RPG Maker 游戏不可感知。
