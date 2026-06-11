# 窗口约束系统设计

## 目标

窗口层只负责窗口本身的约束和菜单命令，渲染层负责把固定游戏画面
放进当前 surface。整数倍缩放不是锁定状态；它只是一次性窗口尺寸命令
或全屏显示模式。

游戏基准尺寸当前为 `640x480`，宽高比为 `4:3`。

## 状态

```rust
aspect_locked: bool
pending_resize: Option<PendingResize>
fullscreen_scale_mode: FullscreenScaleMode

enum FullscreenScaleMode {
    Fit,
    Integer(u32), // 1..=4
}
```

- `aspect_locked`：只在窗口模式生效。开启后，所有窗口 resize 输入都被
  强制 snap 到 `4:3`。
- `pending_resize`：记录一个正在等待系统兑现的 `request_inner_size` 目标尺寸，
  防止 live resize 期间反复发程序化 resize 请求。
- `fullscreen_scale_mode`：只在全屏模式影响渲染 viewport。

不再需要 `scale_locked`。整数倍缩放不持续约束窗口，也不影响
`aspect_locked`。

## 菜单 / 键盘语义

| 操作 | 窗口模式 | 全屏模式 |
|------|----------|----------|
| `Lock Aspect Ratio` / A | 切换 `aspect_locked`，开启时立即把窗口修到 `4:3` | 状态可切换，不请求 windowed fit，也不约束全屏窗口 |
| `Fit` | 显式请求窗口调整到当前尺寸附近的无黑边 `4:3` 尺寸 | 设置 `FullscreenScaleMode::Fit` |
| `1x`-`4x` | 显式请求窗口调整到 `640x480 * n` | 设置 `FullscreenScaleMode::Integer(n)` |
| `0` | 关闭 `aspect_locked`，设置全屏显示模式为 `Fit` | 同左 |
| Alt+Enter | 切换全屏 | 切回窗口 |

窗口模式的 `Fit` 是一次性命令，不打开 `aspect_locked`。例如：

```text
1000x700 -> 933x700   // 窗口偏宽，缩小宽度到 4:3
800x700  -> 800x600   // 窗口偏高，缩小高度到 4:3
```

全屏模式的 `Fit` 复用渲染层现有 letterbox 行为：保持 `4:3`，完整显示，
居中，剩余区域为黑边。不做 stretch、cover，也不裁切。

## 手动拖窗口

窗口模式下：

```text
handle_resize(w, h):
  if pending_resize.target == (w, h):
    pending_resize = None

  decision = classify_resize(w, h)
  if decision == NeedsCorrection:
    c = fit_aspect_size(w, h)
    request_single_resize(c, Coalesced)  // may be suppressed

  graphics.on_resize(w, h)    // always called, even when off-ratio
  refresh_menu_marks()
```

防抖期间如果 `request_single_resize` 被抑制，窗口会暂时留在 off-ratio 尺寸，
但 graphics 层总是获取实际窗口大小，内容不会拉伸。
`about_to_wait` 会在 pending 超时后自动重试修正。

全屏模式下：

```text
handle_resize(w, h):
  if pending_resize.target == (w, h):
    pending_resize = None
  graphics.on_resize(w, h)
  refresh_menu_marks()
```

全屏时忽略 `aspect_locked` 的窗口约束。渲染层仍然按
`fullscreen_scale_mode` 计算游戏 viewport。

## Resize 防抖

macOS 在拖拽和程序化 resize 时会连续发送大量 `Resized` 事件。
窗口层把程序化 resize 分为两类：

- `Coalesced`：自动宽高比修正请求。用于 live resize 输入和
  `about_to_wait` 重试，会被 pending 防抖抑制。
- `Explicit`：用户显式命令。用于窗口模式下的 `Fit`、`1x`-`4x`，以及
  在窗口模式开启 `Lock Aspect Ratio` 时的立即 fit。它可以覆盖正在等待的
  `Coalesced` 请求，避免菜单命令被之前的自动修正吞掉。

`pending_resize` 只能在两种情况下解除：

- 收到目标尺寸的 `Resized(target_w, target_h)`。
- 请求超过短超时（当前实现为 `100ms`），允许下一次修正覆盖旧目标。

```text
request_single_resize(size, mode):
  if mode == Coalesced and pending_resize exists and not timed out:
    return

  pending_resize = (size, now)
  window.request_inner_size(size)
```

不要在任意下一次 `Resized` 到达时清除 pending。macOS live resize 会在
拖拽期间持续发送用户输入尺寸；如果每个输入事件都清除 pending 并立即发新
的 `request_inner_size`，窗口尺寸虽然会被纠正，但事件循环会被 resize 请求
风暴压住，表现为拖拽卡死。

Pending 防抖只暂时抑制重复的自动修正请求。如果 pending 超时且窗口仍为
off-ratio，`about_to_wait` 中的轻量检查会自动重试修正，无需等待下一次
用户拖拽事件。用户随后点击窗口模式 `Fit` 或 `1x`-`4x` 时，显式请求会
替换旧 pending，立即请求目标尺寸。

## Checkmark 同步

窗口模式下，`Fit` 是一次性命令，不是状态；整数倍菜单勾选反映实际窗口尺寸，
不反映历史点击：

- `Lock Aspect Ratio`：`aspect_locked == true` 时打勾。
- `Fit`：始终不打勾，即使刚执行过窗口 fit 或窗口当前正好是无黑边 `4:3`。
- `1x`-`4x`：窗口实际尺寸正好等于 `640x480 * n` 时对应项打勾。
- 窗口不是 `1x`-`4x` 精确整数倍时，`1x`-`4x` 全部不勾。

全屏模式下，菜单勾选反映 `fullscreen_scale_mode`：

- `Fit`：`FullscreenScaleMode::Fit` 时打勾。
- `1x`-`4x`：`FullscreenScaleMode::Integer(n)` 时对应项打勾。
- `Lock Aspect Ratio` 仍然只显示 `aspect_locked` 状态，但不影响全屏窗口。

## 渲染层 viewport

渲染层接收完整 surface 尺寸，并根据显示模式计算游戏 viewport：

```rust
enum ViewportScaleMode {
    Fit,
    Integer(u32),
}
```

- `Fit`：保持 `4:3`，在窗口或全屏 surface 内完整显示，居中，可能有黑边。
- `Integer(n)`：优先使用 `n * 640` 和 `n * 480` 的 viewport，居中，可能有黑边。
  如果当前 surface 小于目标整数 viewport，则降级为 `Fit`，确保 viewport 始终
  位于 render target 内。macOS 原生退出全屏动画期间可能先发较小的真实
  surface resize，graphics 层必须避免生成超出 render target 的 viewport。

窗口模式下的菜单 `Fit` 会先调整窗口本身到无黑边 `4:3` 尺寸，所以渲染层
仍然可以使用 `Fit`。全屏模式下的菜单 `Fit` 不改变显示器尺寸，只改变
viewport 计算策略。

## 约束函数

```text
fit_aspect_size(w, h):
  target = 640 / 480

  if w / h > target:
    return (round(h * target), h)
  else:
    return (w, round(w / target))
```

```text
integer_size(n):
  return (640 * n, 480 * n)
```

```text
window_scale_mark(w, h):
  if w % 640 == 0 and h % 480 == 0 and w / 640 == h / 480:
    n = w / 640
    if 1 <= n <= 4:
      return Integer(n)

  return None
```

## 事件流

菜单 `2x`，窗口模式：

```text
request_single_resize(1280, 960, Explicit)

[Resized(1280, 960)]
  pending_resize = None
  graphics.on_resize(1280, 960)
  refresh_menu_marks() -> 2x checked
```

菜单 `Fit`，窗口模式：

```text
current = 1000x700
fit_aspect_size(current) -> 933x700
request_single_resize(933, 700, Explicit)

[Resized(933, 700)]
  pending_resize = None
  graphics.on_resize(933, 700)
  refresh_menu_marks() -> no scale checked
```

手动拖拽，`aspect_locked == true`：

```text
[Resized(1100, 800)]
  classify_resize -> NeedsCorrection
  request_single_resize(1067, 800, Coalesced)  // 发送修正请求
  graphics.on_resize(1100, 800)      // 图形层使用实际窗口尺寸

[Resized(1067, 800)]（修正到账）
  observe_resized -> pending 清除
  classify_resize -> Proceed
  graphics.on_resize(1067, 800)

若防抖期间修正被抑制，about_to_wait 在超时后自动重试。
```

菜单 `3x`，全屏模式：

```text
fullscreen_scale_mode = Integer(3)
graphics.set_viewport_scale_mode(Integer(3))
refresh_menu_marks() -> 3x checked
```

菜单 `Fit`，全屏模式：

```text
fullscreen_scale_mode = Fit
graphics.set_viewport_scale_mode(Fit)
refresh_menu_marks() -> Fit checked
```
