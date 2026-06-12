# winit 所需做的事情：对照 mkxp-z 源码，按调用顺序

本文对照 mkxp-z 的 `main.cpp` 和 `eventthread.cpp`，列出 winit
二进制入口要做的所有事情，从启动到退出的完整调用序列。

---

## 第一阶段：启动（main 函数，winit 事件循环之前）

### 1. 配置和平台初始化

| mkxp-z（main.cpp） | winit/Rust 等价 |
|---|---|
| `SDL_SetHint(...)` (4 条 hint) | 不需要。winit 有自己的默认行为 |
| `SDL_Init(VIDEO\|GAMECONTROLLER\|TIMER)` | `EventLoop::new()` — winit 自动初始化 |
| `EventThread::allocUserEvents()` | 不需要。winit 没有自定义事件机制，改用共享状态 |
| 设置当前目录 | `set_current_dir(game_path)` |
| `Config::read(argc, argv)` | `mkxp_config::load()` → 直接用现成的 crate |
| `IMG_Init(PNG\|JPG)` | 不需要。`image` crate 解码，不依赖 SDL_image |
| `TTF_Init()` | 不需要。字体渲染用 `rusttype` 或 `ab_glyph`，不依赖 SDL_ttf |
| `Sound_Init()` | 不需要。音频用 `mkxp-audio` crate，不依赖 SDL_sound |
| [Windows] 调试控制台 | 开发阶段不需要，分发版本用 Windows 子系统 |

### 2. 创建窗口

| mkxp-z | winit |
|---|---|
| `SDL_CreateWindow(title, x, y, w, h, flags)` | Builder 模式创建 winit Window |
| `SDL_WINDOW_OPENGL` | 不需要。wgpu 自己管理 GPU 上下文 |
| `SDL_WINDOW_INPUT_FOCUS` | winit 默认 |
| `SDL_WINDOW_ALLOW_HIGHDPI` | winit 默认处理 DPI |
| `SDL_WINDOW_RESIZABLE` | `Window::with_resizable(true)` |
| `SDL_WINDOW_FULLSCREEN_DESKTOP` | `Window::set_fullscreen(Some(Fullscreen::Borderless(None)))` |
| `setupWindowIcon(win)` (Linux) | `Window::set_window_icon(icon)` |

然后从 winit 窗口创建 wgpu surface：

```rust
let surface = instance.create_surface(window.clone())?;
```

### 3. 创建 wgpu 设备（替代 OpenGL 上下文）

| mkxp-z | wgpu |
|---|---|
| `initGL(win, conf)` | 创建 `wgpu::Instance` → `request_adapter()` → `request_device()` |
| `SDL_GL_CreateContext` | wgpu 内部处理 |
| `SDL_GL_MakeCurrent` | wgpu 内部处理 |
| `glGetString(GL_VENDOR/RENDERER/...)` | `adapter.get_info()` — 打印 GPU 信息 |
| 读取 `SDL_GL_GetDrawableSize` | `surface.get_current_texture()` 时自动得到 |

### 4. 查询显示信息

| mkxp-z | winit |
|---|---|
| `SDL_GetCurrentDisplayMode(0, &mode)` | `window.current_monitor().and_then(|m| m.refresh_rate_millihertz())` |
| 用 `mode.refresh_rate` 做垂直同步 | `PresentMode::Fifo`（默认 = vsync） |

### 5. 音频设备

| mkxp-z | mkxp-rs |
|---|---|
| `alcOpenDevice(0)` → `alcCreateContext` | `mkxp_audio::AudioManager::new()` — 已实现 |

### 6. 读取初始窗口尺寸

| mkxp-z | winit |
|---|---|
| `SDL_GetWindowSize(win, &w, &h)` | `window.inner_size()` — 直接读 |
| 通过 `windowSizeMsg.post()` 发给 RGSS 线程 | 不需要发消息。存在 `Shared` 里双方都能读 |

### 7. 启动 Ruby 线程

| mkxp-z | mkxp-rs |
|---|---|
| `SDL_CreateThread(rgssThreadFun)` | `std::thread::spawn(move \|\| { ruby_thread(...) })` |
| RGSS 线程内：`SharedState::initInstance()` | Ruby 线程内：初始化 magnus，加载脚本 |
| 然后 `eventThread.process(rtData)` | 然后 `event_loop.run(...)`（进入第二阶段） |

---

## 第二阶段：事件循环

mkxp-z 用 `while(true) { SDL_WaitEvent(&event); }` 阻塞等事件。
winit 用 `event_loop.run(move |event, elwt| { ... })` 推送事件。

### 8. 窗口事件

#### 8a. 窗口大小改变

**mkxp-z:**
```cpp
case SDL_WINDOWEVENT_SIZE_CHANGED:
    winW = event.window.data1;
    winH = event.window.data2;
    SDL_GL_GetDrawableSize(win, &drwW, &drwH);
    windowSizeMsg.post(Vec2i(winW, winH));
    drawableSizeMsg.post(Vec2i(drwW, drwH));
```

**winit:**
```rust
WindowEvent::Resized(physical_size) => {
    graphics.on_resize(physical_size.width, physical_size.height);
    // 不需要分别通知逻辑尺寸和物理像素——wgpu 的 surface config 处理
}
```

#### 8b. 鼠标进入/离开窗口

**mkxp-z:**
```cpp
case SDL_WINDOWEVENT_ENTER:
    cursorInWindow = true;
    mouseState.inWindow = true;
```

**winit:**
```rust
WindowEvent::CursorEntered { .. } => input.mouse_in_window.store(true, Relaxed);
WindowEvent::CursorLeft { .. } => input.mouse_in_window.store(false, Relaxed);
```

#### 8c. 关闭窗口

**mkxp-z:**
```cpp
case SDL_WINDOWEVENT_CLOSE:
    terminate = true;
```

**winit:**
```rust
WindowEvent::CloseRequested => elwt.exit();
```

#### 8d. 焦点获取/失去

**mkxp-z:**
```cpp
case SDL_WINDOWEVENT_FOCUS_LOST:
    windowFocused = false;
    resetInputStates();  // 清空所有按键状态，防止卡键
```

**winit:**
```rust
WindowEvent::Focused(false) => input.reset_all();
WindowEvent::Focused(true) => { /* 更新光标 */ }
```

### 9. 键盘事件

**mkxp-z:**
```cpp
case SDL_KEYDOWN:
    if (Alt+Enter) toggleFullscreen();
    if (F1) openSettings();
    if (F2) toggleFPS();
    if (F12) { resetting = true; rqReset.set(); }
    keyStates[scancode] = true;

case SDL_KEYUP:
    if (F12) { resetting = false; rqResetFinish.set(); }
    keyStates[scancode] = false;
```

**winit:**
```rust
WindowEvent::KeyboardInput {
    event: KeyEvent { physical_key, state, .. }, ..
} => {
    let pressed = state.is_pressed();

    // 特殊按键——直接在 winit 线程处理
    if pressed {
        match physical_key {
            KeyCode::F12 => outputs.push(WindowOutput::RestartRequested),
            KeyCode::F2  => fps_display_toggle(),
            _ => {}
        }
    }

    // 普通按键——写入共享数组给 Ruby 读
    input.keys[scancode].store(pressed, Relaxed);
}
```

> Alt+Enter 全屏切换：winit 不直接支持。需手动检测
> `Modifiers::ALT` + `KeyCode::Enter`，然后调
> `window.set_fullscreen(Some/None)`。

### 10. 鼠标事件

**mkxp-z:**
```cpp
case SDL_MOUSEBUTTONDOWN: mouseState.buttons[button] = true;
case SDL_MOUSEBUTTONUP:   mouseState.buttons[button] = false;
case SDL_MOUSEMOTION:     mouseState.x = x; mouseState.y = y;
case SDL_MOUSEWHEEL:      SDL_AtomicAdd(&verticalScrollDistance, y);
```

**winit:**
```rust
WindowEvent::MouseInput { button, state, .. } => {
    input.mouse_buttons[btn].store(state.is_pressed(), Relaxed);
}
WindowEvent::CursorMoved { position, .. } => {
    input.mouse_x.store(position.x as i32, Relaxed);
    input.mouse_y.store(position.y as i32, Relaxed);
}
WindowEvent::MouseWheel { delta, .. } => {
    // delta: LineDelta(x, y) 或 PixelDelta
    input.scroll_y.fetch_add(lines as i32, Relaxed);
}
```

### 11. 手柄事件

**mkxp-z:** `SDL_GameController` API

**winit:** winit 0.30 不内置。用 `gilrs` crate：

```rust
// 在 AboutToWait 或单独轮询
while let Some(ev) = gilrs.next_event() {
    match ev.event {
        EventType::ButtonPressed(btn, _) => input.ctrl.buttons[btn] = true,
        EventType::ButtonReleased(btn, _) => input.ctrl.buttons[btn] = false,
        EventType::AxisChanged(axis, val, _) => input.ctrl.axes[axis] = val,
        _ => {}
    }
}
```

### 12. 触摸事件

**mkxp-z:** `SDL_FINGERDOWN/MOTION/UP`

**winit:**
```rust
WindowEvent::Touch(Touch { phase, location, id, .. }) => {
    match phase {
        TouchPhase::Started => input.touch[id].down = true,
        TouchPhase::Moved => {
            input.touch[id].x = location.x;
            input.touch[id].y = location.y;
        }
        TouchPhase::Ended | TouchPhase::Cancelled => input.touch[id].reset(),
    }
}
```

v1 无需实现（桌面 RPG Maker 几乎不需要触摸），保留骨架即可。

### 13. 窗口操作请求（替代 mkxp-z 的 SDL 用户事件）

mkxp-z 中 Ruby 线程通过 `SDL_PushEvent()` 发用户事件给主线程
来触发窗口操作。winit 里全是**直接函数调用**，不需要事件中转：

| mkxp-z 用户事件 | 谁发的 | winit 等价 |
|---|---|---|
| `REQUEST_SETFULLSCREEN` | `Graphics.fullscreen = true` | `window.set_fullscreen(Some(mode))` |
| `REQUEST_WINRESIZE` | `Graphics.resize_screen(w,h)` | `window.request_inner_size(size)` |
| `REQUEST_WINREPOSITION` | 移动窗口 | `window.set_outer_position(pos)` |
| `REQUEST_WINCENTER` | `Graphics.center` | 计算屏幕中心 + `set_outer_position` |
| `REQUEST_WINRENAME` | 改标题 | `window.set_title(title)` |
| `REQUEST_SETCURSORVISIBLE` | `Graphics.show_cursor =` | `window.set_cursor_visible(show)` |
| `REQUEST_MESSAGEBOX` | `msgbox("text")` | `rfd::MessageDialog` 或自定义 |
| `UPDATE_FPS` | 每帧 | `window.set_title(&format!(...))` |
| `UPDATE_SCREEN_RECT` | 游戏区域改变 | 不需要 |

### 14. 帧渲染

| mkxp-z | mkxp-rs 目标设计 |
|---|---|
| RGSS 线程调 `Graphics::update()` | script thread 在 `FrameSync` 设置 ready 并阻塞 |
| `fpsLimiter.delay()` | render thread 持有 `next_frame_at` 做 FPS gate |
| `SDL_GL_SwapWindow` | render thread 调 `GraphicsState::update()` / `frame.present()` |

早期迁移草案曾计划在 winit `AboutToWait` 中渲染。macOS 输入法快速切换会让主线程长时间不回到
winit handler，因此目标设计改为独立 render host thread。完整设计见
[`FRAME_LOOP_DESIGN.md`](FRAME_LOOP_DESIGN.md)。

### 15. 应用前后台

mkxp-z 通过 `SDL_APP_WILLENTERBACKGROUND` 暂停音频。
桌面平台通常不需要。v1 可忽略。

---

## 第三阶段：退出

| mkxp-z | winit |
|---|---|
| `WINDOWEVENT_CLOSE` → `terminate=true` | `elwt.exit()` |
| `rqTerm.set()` → 等 `rqTermAck` | `signals.shutdown = true` → `ruby_thread.join()` |
| `SDL_WaitThread(rgssThread)` | `thread.join()` |
| `SDL_GameControllerClose` | gilrs 自动管理 |
| `alcCloseDevice` → `SDL_DestroyWindow` → `SDL_Quit` | winit/wgpu 自动 Drop |

---

## 总结：winit 要做的完整清单

```
启动阶段：
  □ 1.  加载 Config
  □ 2.  创建 EventLoop
  □ 3.  创建 winit Window（标题、尺寸、resizable、fullscreen）
  □ 4.  设置窗口图标（Linux）
  □ 5.  创建 wgpu Instance → Adapter → Device + Queue
  □ 6.  从 Window 创建 wgpu Surface
  □ 7.  创建 GraphicsState（传入 Device/Queue/Surface）
  □ 8.  创建 AudioManager
  □ 9.  创建 Shared { graphics, input, signals, frame_sync, config }
  □ 10. 启动 render thread（传入 Arc<Shared> + RenderCommand receiver）
  □ 11. 启动 Ruby 线程（传入 Arc<Shared>）

事件循环阶段：
  □ 12. WindowEvent::Resized → RenderCommand::SurfaceResized
  □ 13. WindowEvent::CloseRequested → elwt.exit()
  □ 14. WindowEvent::Focused(false) → 清空输入状态
  □ 15. WindowEvent::KeyboardInput
         - Alt+Enter → 切换全屏
         - F12 → signals.reset = true
         - F2 → 切换 FPS 显示
         - 其他 → input.keys[scancode] = pressed
  □ 16. WindowEvent::CursorMoved / MouseInput / MouseWheel → input 原子变量
  □ 17. [可选] gilrs 手柄事件 → input.controller
  □ 18. [可选] Touch 事件
  □ 19. AboutToWait：drain menu/window housekeeping only，不执行每帧 render

退出阶段：
  □ 20. signals.shutdown = true
  □ 21. RenderCommand::Shutdown + frame_sync.wake_all()
  □ 22. ruby_thread.join()
  □ 23. render_thread.join()
  □ 24. graphics/runtime 先 drop，window 后 drop
```

---

## mkxp-z 有而 winit 不需要做的

| mkxp-z | 为什么 winit 不需要 |
|---|---|
| `EventThread::allocUserEvents()` | 自定义事件机制。窗口操作直接函数调用 |
| `windowSizeMsg` 跨线程消息 | winit 和 Ruby 共享同一套状态，无消息传递 |
| `SDL_GL_MakeCurrent` 上下文切换 | wgpu 的 Device 是 Send+Sync，不需要切换线程 |
| `SyncPoint` 暂停/恢复线程 | 桌面 v1 不需要后台暂停 |
| `SDL_GetDrawableSize` vs `SDL_GetWindowSize` | wgpu 统一处理物理/逻辑像素 |
| `IMG_Init` / `TTF_Init` / `Sound_Init` | 分别被 `image` / `ab_glyph` / `mkxp-audio` 替代 |
