# 窗口约束系统设计

本文是窗口行为规范。所有权和线程边界见
[`WINDOW_CONTROLLER_DESIGN.md`](WINDOW_CONTROLLER_DESIGN.md)，frame loop 见
[`FRAME_LOOP_DESIGN.md`](FRAME_LOOP_DESIGN.md)。

## 目标

窗口层负责窗口尺寸、菜单命令、全屏状态和宽高比约束。渲染层负责把固定游戏画面
放进当前 surface。

游戏逻辑尺寸来自启动时解析出的 `RuntimeConfig.game_size`，默认值为 `640x480`。
窗口约束和整数倍缩放都使用这个配置值，而不是窗口模块内的全局常量。

## 核心状态

```rust
aspect_locked: bool
pending_resize: Option<PendingResize>
fullscreen_scale_mode: FullscreenScaleMode

enum FullscreenScaleMode {
    Fit,
    Integer(u32), // 1..=4
}
```

- `aspect_locked`：只约束窗口模式下的用户 resize。开启时，窗口 resize 会被修正
  到 `4:3`。
- `pending_resize`：记录已经发送给平台、尚未被 `Resized(target)` 确认的
  `request_inner_size`，用于防止 live resize 请求风暴。
- `fullscreen_scale_mode`：只在全屏模式影响渲染 viewport。

不再存在持续的 `scale_locked`。整数倍缩放是一次性窗口尺寸命令，或全屏 viewport
模式。

## 菜单和快捷键语义

| 操作 | 窗口模式 | 全屏模式 |
|---|---|---|
| `Lock Aspect Ratio` | 切换 `aspect_locked`；开启时立即 fit 当前窗口到 `game_size` 宽高比 | 只切换状态，不约束全屏 surface |
| `Fit` | 一次性请求窗口调整到当前尺寸附近的无黑边 `game_size` 宽高比尺寸 | 设置 `FullscreenScaleMode::Fit` |
| `1x`-`4x` | 一次性请求窗口调整到 `game_size * n` | 设置 `FullscreenScaleMode::Integer(n)` |
| `Alt+Enter` | 进入 borderless fullscreen | 退出全屏 |
| `F12` | reset 启用时请求脚本 restart | 同左 |
| `Game > Restart` | reset 启用时请求脚本 restart | 同左 |
| `Game > Quit` / 关闭窗口 | 请求退出 | 请求退出 |

窗口模式的 `Fit` 不是状态，也不会自动开启 `aspect_locked`：

```text
1000x700 -> 933x700
800x700  -> 800x600
```

全屏模式的 `Fit` 复用渲染层 letterbox 行为：保持 `game_size` 宽高比、完整显示、
居中、剩余区域为黑边。不 stretch、不 cover、不裁切。

## 手动拖窗口

窗口模式：

```text
on Resized(w, h):
  resize_requests.observe_resized(w, h)

  if aspect_locked and fit_aspect_size(w, h) != (w, h):
    request_single_resize(fit_aspect_size(w, h), Coalesced)

  emit SurfaceResized(w, h)
  refresh_menu_marks()
```

即使窗口短暂停在 off-ratio，`SurfaceResized` 仍然使用真实尺寸。render thread
用真实 surface 尺寸 reconfigure，避免画面和 surface config 不一致。

全屏模式：

```text
on Resized(w, h):
  resize_requests.observe_resized(w, h)
  emit SurfaceResized(w, h)
  refresh_menu_marks()
```

全屏时忽略 `aspect_locked` 对窗口尺寸的约束；渲染层按
`fullscreen_scale_mode` 计算 viewport。

## Resize 防抖

程序化 resize 分两类：

- `Coalesced`：自动宽高比修正。用于 live resize 和 `about_to_wait` 重试；如果已有
  未超时 pending 请求，会被抑制。
- `Explicit`：用户显式命令。用于窗口模式 `Fit`、`1x`-`4x`，以及开启
  `Lock Aspect Ratio` 时的立即 fit；可以覆盖 pending 的自动修正。

pending 只在两种情况下解除：

- 收到目标尺寸的 `Resized(target_w, target_h)`。
- 请求超过短超时（当前实现为 `100ms`），允许下一次修正覆盖旧目标。

```text
request_single_resize(size, mode):
  if mode == Coalesced and pending exists and not timed out:
    return

  pending = (size, now)
  window.request_inner_size(size)
```

不要在任意下一次 `Resized` 到达时清除 pending。macOS live resize 会持续发送用户
拖拽尺寸；如果每个输入尺寸都清 pending 并立即发新请求，会形成 resize 请求风暴。

`about_to_wait` 中保留轻量重试：如果 pending 超时且当前窗口仍 off-ratio，
controller 会再次发 `Coalesced` 修正。

## Checkmark 同步

窗口模式下：

- `Lock Aspect Ratio`：`aspect_locked == true` 时勾选。
- `Fit`：始终不勾选，因为它是命令，不是状态。
- `1x`-`4x`：当前窗口真实尺寸正好等于 `game_size * n` 时勾选对应项。
- 其他窗口尺寸：`1x`-`4x` 全不勾选。

全屏模式下：

- `Fit`：`FullscreenScaleMode::Fit` 时勾选。
- `1x`-`4x`：`FullscreenScaleMode::Integer(n)` 时勾选对应项。
- `Lock Aspect Ratio` 仍显示 `aspect_locked` 状态，但不影响全屏 surface。

退出全屏后，`Fit` 不应继续勾选；窗口模式 `Fit` 没有持久状态。

## 渲染层 viewport

render thread 接收完整 surface 尺寸和 viewport mode：

```rust
enum ViewportScaleMode {
    Fit,
    Integer(u32),
}
```

- `Fit`：保持 `game_size` 宽高比，在 surface 内完整显示并居中。
- `Integer(n)`：优先使用 `n * game_size.w` 和 `n * game_size.h` 的 viewport；如果 surface 小于目标
  viewport，则降级到 `Fit`，确保 viewport 不越界。

窗口模式菜单 `Fit` 会调整窗口本身到无黑边的 `game_size` 宽高比尺寸，所以 graphics
仍可保持 `ViewportScaleMode::Fit`。全屏菜单 `Fit` 不改变显示器尺寸，只改变
viewport 计算策略。

## 约束函数

```text
fit_aspect_size(w, h, game_w, game_h):
  target = game_w / game_h
  if w / h > target:
    return (round(h * target), h)
  else:
    return (w, round(w / target))
```

```text
integer_size(n, game_w, game_h):
  return (game_w * n, game_h * n)
```

```text
window_scale_mark(w, h, game_w, game_h):
  if w % game_w == 0 and h % game_h == 0 and w / game_w == h / game_h:
    n = w / game_w
    if 1 <= n <= 4:
      return Integer(n)
  return None
```

## 事件流示例

窗口模式菜单 `2x`：

```text
request_single_resize(1280, 960, Explicit)

Resized(1280, 960)
  pending = None
  emit SurfaceResized(1280, 960)
  refresh_menu_marks() -> 2x checked
```

窗口模式菜单 `Fit`：

```text
current = 1000x700
fit_aspect_size(current) -> 933x700
request_single_resize(933, 700, Explicit)

Resized(933, 700)
  pending = None
  emit SurfaceResized(933, 700)
  refresh_menu_marks() -> no scale checked
```

手动拖拽且 `aspect_locked == true`：

```text
Resized(1100, 800)
  classify_resize -> NeedsCorrection
  request_single_resize(1067, 800, Coalesced)
  emit SurfaceResized(1100, 800)

Resized(1067, 800)
  pending = None
  classify_resize -> Proceed
  emit SurfaceResized(1067, 800)
```

全屏菜单 `3x`：

```text
fullscreen_scale_mode = Integer(3)
emit ViewportScaleModeChanged(Integer(3))
refresh_menu_marks() -> 3x checked
```

脚本 restart：

```text
F12 or Game > Restart
  if enable_reset:
    emit RestartRequested

App
  requests runtime restart
  wakes blocked script
  joins old script thread
  spawns a fresh E::default()
```
