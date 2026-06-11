# 窗口约束系统设计

## 目标

窗口层只负责窗口本身的约束和菜单命令，渲染层负责把固定游戏画面
放进当前 surface。整数倍缩放不是锁定状态；它只是一次性窗口尺寸命令
或全屏显示模式。

游戏基准尺寸当前为 `640x480`，宽高比为 `4:3`。

## 状态

```rust
aspect_locked: bool
resize_in_progress: bool
fullscreen_scale_mode: FullscreenScaleMode

enum FullscreenScaleMode {
    Fit,
    Integer(u32), // 1..=4
}
```

- `aspect_locked`：只在窗口模式生效。开启后，所有窗口 resize 输入都被
  强制 snap 到 `4:3`。
- `resize_in_progress`：防止同一时间发出多个 `request_inner_size` 请求。
- `fullscreen_scale_mode`：只在全屏模式影响渲染 viewport。

不再需要 `scale_locked`。整数倍缩放不持续约束窗口，也不影响
`aspect_locked`。

## 菜单 / 键盘语义

| 操作 | 窗口模式 | 全屏模式 |
|------|----------|----------|
| `Lock Aspect Ratio` / A | 切换 `aspect_locked`，开启时立即把窗口修到 `4:3` | 状态可切换但不约束全屏窗口 |
| `Fit` | 请求窗口调整到当前尺寸附近的无黑边 `4:3` 尺寸 | 设置 `FullscreenScaleMode::Fit` |
| `1x`-`4x` | 请求窗口调整到 `640x480 * n` | 设置 `FullscreenScaleMode::Integer(n)` |
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
  resize_in_progress = false

  if aspect_locked:
    c = fit_aspect_size(w, h)
    if c != (w, h):
      request_single_resize(c)
      refresh_menu_marks()
      return

  graphics.on_resize(w, h)
  refresh_menu_marks()
```

全屏模式下：

```text
handle_resize(w, h):
  resize_in_progress = false
  graphics.on_resize(w, h)
  refresh_menu_marks()
```

全屏时忽略 `aspect_locked` 的窗口约束。渲染层仍然按
`fullscreen_scale_mode` 计算游戏 viewport。

## Resize 防抖

macOS 在拖拽和程序化 resize 时会连续发送大量 `Resized` 事件。
窗口层只允许同一时间有一个程序化 resize 请求在飞。

```text
request_single_resize(size):
  if resize_in_progress:
    return

  resize_in_progress = true
  window.request_inner_size(size)
```

下一次 `Resized` 到达时，无论尺寸是否与请求完全一致，都清除
`resize_in_progress`。如果 `aspect_locked` 仍然发现尺寸不合规，可以再发
下一次修正请求。

## Checkmark 同步

窗口模式下，菜单勾选反映实际窗口尺寸，不反映历史点击：

- `Lock Aspect Ratio`：`aspect_locked == true` 时打勾。
- `Fit`：窗口是 `4:3`，但不是 `1x`-`4x` 精确整数倍时打勾。
- `1x`-`4x`：窗口实际尺寸正好等于 `640x480 * n` 时对应项打勾。
- 窗口偏离 `4:3` 时，`Fit` 和 `1x`-`4x` 全部不勾。

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
- `Integer(n)`：使用 `n * 640` 和 `n * 480` 的 viewport，居中，可能有黑边。

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

  if w * 480 == h * 640:
    return Fit

  return None
```

## 事件流

菜单 `2x`，窗口模式：

```text
request_single_resize(1280, 960)

[Resized(1280, 960)]
  resize_in_progress = false
  graphics.on_resize(1280, 960)
  refresh_menu_marks() -> 2x checked
```

菜单 `Fit`，窗口模式：

```text
current = 1000x700
fit_aspect_size(current) -> 933x700
request_single_resize(933, 700)

[Resized(933, 700)]
  resize_in_progress = false
  graphics.on_resize(933, 700)
  refresh_menu_marks() -> Fit checked
```

手动拖拽，`aspect_locked == true`：

```text
[Resized(1100, 800)]
  fit_aspect_size(1100, 800) -> 1067x800
  request_single_resize(1067, 800)

[Resized(1067, 800)]
  graphics.on_resize(1067, 800)
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
