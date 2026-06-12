# mkxp-rs 配置系统

## 概述

mkxp-rs 从 5 个来源读取配置，按优先级合并。高优先级覆盖低优先级。

```
MKXP_* 环境变量              最高优先级
    |
--xxx 命令行参数
    |
~/.config/mkxp-rs/mkxp.ron   用户级配置
    |
游戏目录/mkxp.ron             引擎配置
    |
游戏目录/Game.ini             游戏元数据
```

`mkxp-config` crate 负责读取和合并，生成 `Config` struct 供其他 crate 使用。

用到的库：配置文件格式使用 `ron` + `serde`，Game.ini 解析使用 `rust-ini`，命令行参数解析使用 `clap`。

参考示例位于 `crates/mkxp-config/` 目录下：`mkxp.ron` 和 `Game.ini`。

---

## 引擎配置 (mkxp.ron)

引擎配置使用 [RON](https://github.com/ron-rs/ron) 格式。所有字段可选，缺失时使用 Rust `Default` 值。

### ruby

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `rgss_version` | `"1"` `"2"` `"3"` | `"3"` | 目标 RGSS 版本。1 对应 XP，2 对应 VX，3 对应 VX Ace。 |
| `preload_scripts` | `[String]` | `[]` | 在游戏脚本执行之前加载的 Ruby 脚本列表。 |
| `postload_scripts` | `[String]` | `[]` | 在 rgss_main 之前加载的脚本（仅 RGSS3 生效）。 |
| `custom_script` | `Option<String>` | `None` | 如果设置，仅执行此脚本文件而不加载完整游戏。 |
| `launch_args` | `[String]` | `[]` | 转发给 Ruby 脚本 `ARGV` 的参数。 |
| `use_script_names` | `bool` | `true` | 是否在 Ruby 错误回溯中显示脚本文件名而非内部编号。 |

### window

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `title` | `String` | `""` | 窗口标题。为空时使用 Game.ini 的 Title 字段值。 |
| `size` | `(i32, i32)` | `(640, 480)` | 逻辑分辨率（像素）。`(0, 0)` 表示使用该 RGSS 版本的默认分辨率。 |
| `fullscreen` | `bool` | `false` | 是否以全屏模式启动。运行时可用 Alt+Enter 切换，不受此设置影响。 |
| `resizable` | `bool` | `true` | 是否允许用户拖拽窗口边缘改变尺寸。 |
| `fixed_aspect_ratio` | `bool` | `true` | 窗口尺寸改变时是否保持游戏画面宽高比，多余空间以黑边填充。 |
| `integer_scaling` | `bool` | `false` | 是否以整数倍缩放画面后再填充剩余空间。 |
| `frame_skip` | `bool` | `false` | 当引擎渲染速度落后于帧计划时是否跳过当前帧。 |

### graphics

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `vsync` | `bool` | `true` | 是否等待显示器垂直同步信号后再交换缓冲区，以消除画面撕裂。 |
| `sync_to_refresh_rate` | `bool` | `false` | 是否将帧时序同步到显示器刷新率，并将真实帧率报告回 Ruby 脚本。如果无法检测刷新率则强制禁用。 |
| `frame_rate` | `u32` | `60` | 帧率上限。运行时会将配置值限制在 `1..=240`。 |
| `scale_mode` | ScaleMode | `"bilinear"` | 默认缩放算法，作用于画面放大、缩小和位图缩放。可选 `"nearest"` `"bilinear"` `"bicubic"` `"lanczos3"` `"xbrz"`。 |
| `scale_up` | `Option<ScaleMode>` | `None` | 覆写画面放大算法。`None` 表示跟随 `scale_mode`。 |
| `scale_down` | `Option<ScaleMode>` | `None` | 覆写画面缩小算法。 |
| `bitmap_scale_up` | `Option<ScaleMode>` | `None` | 覆写位图放大算法。 |
| `bitmap_scale_down` | `Option<ScaleMode>` | `None` | 覆写位图缩小算法。 |
| `mipmaps` | `bool` | `false` | 缩小时是否启用 mipmap 插值。仅当 `scale_down` 为 bilinear 时生效。 |
| `bicubic_sharpness` | `u32` | `100` | Bicubic 缩放算法的锐度参数，取值范围 0 到 200。 |
| `xbrz_factor` | `f64` | `4.0` | xBRZ 算法的缩放倍率。 |
| `hires.enabled` | `bool` | `false` | 是否启用高分辨率纹理替换，开启后引擎在 `Hires` 子目录中查找同名高分辨率图片。 |
| `hires.factor` | `f64` | `4.0` | 高分辨率纹理相对于原始位图的缩放倍率。 |
| `enable_blitting` | `bool` | `true` | 是否使用硬件 framebuffer blitting。使用非 nearest/bilinear 缩放算法时强制禁用。 |
| `max_texture_size` | `u32` | `0` | 纹理尺寸上限。`0` 表示使用硬件支持的最大值。 |
| `pixel_snap` | `bool` | `false` | 是否将渲染锁定到整数像素边界。关闭后 Sprite 具有子像素精度，移动更平滑。 |

### paths

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `game_folder` | `Option<String>` | `None` | 游戏根目录的路径。`None` 表示当前工作目录。 |
| `rtp` | `[String]` | `[]` | RPG Maker RTP 资源包路径列表，支持目录、zip 文件和加密档案。 |
| `patches` | `[String]` | `[]` | 补丁或 Mod 的加载路径列表，搜索优先级高于 `game_folder`。 |
| `icon_path` | `Option<String>` | `None` | 自定义窗口图标路径（仅 Linux）。`None` 使用内置默认图标。 |

### fonts

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `default_family` | `String` | `"Arial"` | 当脚本请求的字体不存在时使用的默认字体族。 |
| `scale` | `f64` | `1.0` | 全局字体缩放倍率。 |
| `hinting` | Hinting | `"none"` | 字体微调级别，可选 `"normal"` `"light"` `"mono"` `"none"`。RGSS 不使用微调，选 `"none"` 最接近原版效果。 |
| `kerning` | `bool` | `false` | 是否启用字符间距调整。RGSS 不使用 kerning。 |
| `outline_crop` | `bool` | `true` | 是否裁剪带描边文字的顶部一行和左侧一列像素。匹配 RGSS 行为。 |
| `substitutions` | `[{from, to}]` | `[]` | 字体替换规则列表，将请求的字体族映射到实际渲染的字体族。 |
| `solid` | `[String]` | `[]` | 不使用 alpha 混合渲染的字体族列表，引擎会缓存字形位图以提升性能。 |
| `height_reporting` | HeightMode | `"nominal"` | 文字高度计算方式：`"nominal"` 使用字体度量（RGSS 行为），`"rendered"` 使用实际像素高度。 |

### input

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `key_bindings` | `[{key, action}]` | `[]` | 自定义键盘绑定列表，将物理键名映射到游戏动作名。 |
| `gamepad_bindings` | `[{key, action}]` | `[]` | 自定义手柄绑定列表，格式与键盘绑定相同。 |
| `binding_names` | `{action: name}` | `{}` | F1 按键设置菜单中显示的动作名称。 |
| `enable_reset` | `bool` | `true` | 是否允许按 F12 重置游戏。 |
| `enable_settings` | `bool` | `true` | 是否允许按 F1 打开按键设置菜单。 |

### audio

> **与 mkxp-z 的差异:** 移除了 `midi_synth` 配置项（rustysynth 是唯一的 MIDI 合成器）。`soundfont` 重命名为 `midi_soundfont`。

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `master_volume` | `f64` | `1.0` | 主音量倍率，取值范围 0.0 到 1.0。 |
| `bgm_volume` | `f64` | `1.0` | BGM（背景音乐）音量倍率。 |
| `se_volume` | `f64` | `1.0` | SE（音效）音量倍率。 |
| `bgs_volume` | `f64` | `1.0` | BGS（背景环境音）音量倍率。 |
| `me_volume` | `f64` | `1.0` | ME（音乐效果）音量倍率。 |
| `midi_soundfont` | `Option<String>` | `None` | MIDI 播放的 SoundFont 文件路径。为空时 MIDI 静音播放。 |
| `midi_chorus` | `bool` | `false` | 是否启用 MIDI 合唱效果（rustysynth）。 |
| `midi_reverb` | `bool` | `false` | 是否启用 MIDI 混响效果（rustysynth）。 |
| `se_source_count` | `u32` | `6` | SE 同时播放的数量上限，最大 64。 |
| `bgm_track_count` | `u32` | `1` | BGM 同时播放的轨道数上限，最大 16。 |

### debug

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `mode` | `bool` | `false` | 是否启用调试日志，将引擎内部信息打印到标准输出。 |
| `console` | `bool` | `false` | 是否弹出独立控制台窗口用于日志输出（仅 Windows）。 |
| `show_fps` | FpsDisplay | `"none"` | 帧率显示位置，可选 `"none"` `"titlebar"` `"console"` `"both"`。 |
| `log_level` | LogLevel | `None` | 覆盖日志详细级别：`"error"`、`"warn"`、`"info"`、`"debug"`、`"trace"`。设置后优先级高于 `mode` 字段。`None` 表示使用 `mode` 作为快捷开关（`false` = info，`true` = debug）。 |

**日志级别优先级**：`log_level` 提供细粒度控制，独立于 `mode` 开关。当 `log_level` 有值时（如 `"trace"`），它始终生效。当 `log_level` 为 `None` 时，`mode` 作为快捷开关：`false` → `info`，`true` → `debug`。此设计在保持向后兼容的同时，允许 CI / 开发环境单独调整日志输出级别。

---

## Game.ini

`Game.ini` 是 RPG Maker 自动生成的 INI 文件。mkxp-z 只读取其中两个字段（源码 `config.cpp:408-409`），mkxp-rs 相同。

`Title` 字段提供游戏名称，当 `window.title` 为空时用作窗口标题。`Scripts` 字段指定 Ruby 脚本档案的路径。

mkxp-z 不使用 `Library` 字段检测 RGSS 版本。它通过 `Scripts` 的文件扩展名判断：`.rxdata` 表示 RGSS1，`.rvdata` 表示 RGSS2，`.rvdata2` 表示 RGSS3。RTP 路径从 `mkxp.json` 加载，不从 `Game.ini` 读取。`Library` 和 `RTP` 是 RPG Maker 编辑器的保留字段，运行时会忽略。

示例：
```ini
[Game]
Title=我的游戏
Scripts=Data\Scripts.rvdata
Library=RGSS300.dll
RTP=Standard
```

---

## 环境变量

环境变量使用 `MKXP_` 前缀，以 `__`（双下划线）作为层级分隔符。例如 `MKXP_WINDOW__TITLE` 映射到 `window.title`。在所有配置来源中优先级最高。布尔值使用 `"1"` 表示 true，`"0"` 表示 false。

mkxp-z 仅定义了 3 个环境变量：`MKXPZ_WINDOWS_CONSOLE`（启用控制台窗口）、`MKXPZ_MACOS_METAL`（强制 Metal 渲染器）和 `MKXPZ_FOLDER_SELECT`（macOS 上显示文件夹选择器），全部是平台特定功能。mkxp-rs 定义了自己的全套环境变量，覆盖常用启动参数。

| 变量 | 覆盖配置项 |
|------|-----------|
| `MKXP_RUBY__RGSS_VERSION` | `ruby.rgss_version` |
| `MKXP_RUBY__CUSTOM_SCRIPT` | `ruby.custom_script` |
| `MKXP_WINDOW__TITLE` | `window.title` |
| `MKXP_WINDOW__SIZE` | `window.size`（格式 `640x480`） |
| `MKXP_WINDOW__FULLSCREEN` | `window.fullscreen` |
| `MKXP_WINDOW__RESIZABLE` | `window.resizable` |
| `MKXP_GRAPHICS__SCALE_MODE` | `graphics.scale_mode` |
| `MKXP_GRAPHICS__VSYNC` | `graphics.vsync` |
| `MKXP_GRAPHICS__FRAME_RATE` | `graphics.frame_rate` |
| `MKXP_PATHS__GAME_FOLDER` | `paths.game_folder` |
| `MKXP_FONTS__SCALE` | `fonts.scale` |
| `MKXP_FONTS__HINTING` | `fonts.hinting` |
| `MKXP_FONTS__KERNING` | `fonts.kerning` |
| `MKXP_FONTS__OUTLINE_CROP` | `fonts.outline_crop` |
| `MKXP_AUDIO__MASTER_VOLUME` | `audio.master_volume` |
| `MKXP_AUDIO__BGM_VOLUME` | `audio.bgm_volume` |
| `MKXP_AUDIO__MIDI_SOUNDFONT` | `audio.midi_soundfont` |
| `MKXP_DEBUG__MODE` | `debug.mode` |
| `MKXP_DEBUG__CONSOLE` | `debug.console` |
| `MKXP_DEBUG__SHOW_FPS` | `debug.show_fps` |
| `MKXP_DEBUG__LOG_LEVEL` | `debug.log_level` |

---

## 命令行参数

命令行参数使用 `--kebab-case` 格式，优先级仅次于环境变量。

mkxp-z 只识别三个参数：`debug`、`test` 和 `btest`（源码 `config.cpp:225-235`）。这些是 RPG Maker XP 的编辑器集成标记，其他参数全部转发给 Ruby `ARGV`。mkxp-rs 定义了与上节环境变量覆盖范围相同的命令行参数。

| 参数 | 覆盖配置项 |
|------|-----------|
| `--rgss-version` | `ruby.rgss_version` |
| `--custom-script <path>` | `ruby.custom_script` |
| `--game-folder <path>` | `paths.game_folder` |
| `--window-title` | `window.title` |
| `--window-size <WxH>` | `window.size` |
| `--fullscreen` | `window.fullscreen`（flag，不需参数） |
| `--no-resizable` | `window.resizable`（flag） |
| `--scale-mode` | `graphics.scale_mode` |
| `--no-vsync` | `graphics.vsync`（flag） |
| `--frame-rate <n>` | `graphics.frame_rate` |
| `--font-scale <n>` | `fonts.scale` |
| `--font-hinting` | `fonts.hinting` |
| `--no-kerning` | `fonts.kerning`（flag） |
| `--no-outline-crop` | `fonts.outline_crop`（flag） |
| `--master-volume <n>` | `audio.master_volume` |
| `--bgm-volume <n>` | `audio.bgm_volume` |
| `--midi-soundfont <path>` | `audio.midi_soundfont` |
| `--debug` | `debug.mode`（flag） |
| `--console` | `debug.console`（flag） |
| `--show-fps` | `debug.show_fps` |
| `--log-level <level>` | `debug.log_level` |

示例：

```bash
mkxp-rs --rgss-version 3 --fullscreen --show-fps titlebar
mkxp-rs --debug --frame-rate 30
mkxp-rs --custom-script benchmark.rb
```

---

## 与 mkxp-z 的差异

**格式。** mkxp-z 使用 JSON5（`mkxp.json`）；mkxp-rs 使用 RON（`mkxp.ron`）。

**结构。** mkxp-z 将所有配置键平铺在 JSON 顶层，约 60 个键在扁平命名空间中。mkxp-rs 将其分组为 8 个 section：`ruby`、`window`、`graphics`、`paths`、`fonts`、`input`、`audio`、`debug`。

**缩放参数。** mkxp-z 使用 6 个独立参数控制画面和位图缩放（`smoothScaling`、`smoothScalingDown`、`bitmapSmoothScaling`、`bitmapSmoothScalingDown`、`smoothScalingMipmaps`、`bicubicSharpness`）。mkxp-rs 使用层级覆写机制：设置 `scale_mode` 作为默认值，需要精细控制时再用 `scale_up`、`scale_down`、`bitmap_scale_up`、`bitmap_scale_down` 单独覆写。

**删除的配置项。** 以下 mkxp-z 的配置项在 mkxp-rs 中不存在：

| 项 | 原因 |
|----|------|
| `JITEnable` `YJITEnable` `JITMaxCache` `JITMinCalls` `JITVerboseLevel` | Ruby JIT 配置通过 Ruby 自身的环境变量控制，不应在引擎配置文件中设置。 |
| `preferMetalRenderer` | wgpu 自动选择渲染后端。 |
| `subImageFix` | 针对某款特定旧 GPU 型号的 workaround。 |
| `anyAltToggleFS` | 允许左或右侧 Alt 键配合 Enter 切换全屏。Alt+Enter 本身已是全局快捷键。 |
| `execName` | mkxp-z 需要此字段处理 Game.exe 被重命名的情况，mkxp-rs 无此历史包袱。 |
| `rubyLoadpath` | 已由静态链接或 `bundle install --standalone` 替代。 |
| `pathCache` `allowSymlinks` | 属于 `mkxp-filesystem` 应处理的文件系统实现细节。 |
| `dataPathOrg` `dataPathApp` | mkxp-z 用此构造 XDG 数据路径，mkxp-rs 直接使用标准 XDG 约定。 |
| `editor` | RPG Maker XP 编辑器集成标记，mkxp-rs 不需要。 |
| `titleLanguage` | 控制窗口标题语言，极少使用。 |
| `manualFolderSelect` | 启动时显示 macOS 游戏文件夹选择器，极少使用。 |
| `dumpAtlas` | 导出 tile atlas 的调试工具，推迟到后续开发阶段。 |

**新增的配置项。** 以下配置项在 mkxp-rs 中存在但 mkxp-z 没有对应项：

| 项 | 说明 |
|----|------|
| `graphics.pixel_snap` | 控制渲染是否锁定到整数像素。关闭后 Sprite 具有子像素精度。 |
| `audio.master_volume` 及各通道音量 | 各音频通道的独立音量控制，mkxp-z 依赖系统混音器调整音量。 |
