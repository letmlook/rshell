# Rust 原生 GUI 与终端实现选型调研

> 调研日期：2026-10-08  
> 范围：替换 RShell 当前 Tauri/Vue/xterm.js 桌面 UI；只使用项目官方仓库、官方文档和源码作为依据。

## 结论

建议把第一轮技术验证从“只验证 egui + 自绘终端”调整为两条候选路线：

1. **主验证：Iced + `iced_term`**。这是当前候选中最接近可嵌入终端控件的方案。`iced_term` 已发布到 crates.io，当前仓库版本为 0.8.0，提供多实例、键鼠输入、鼠标模式、resize、滚动、选区、超链接，并声明在 Windows、macOS、Linux 上测试过；底层使用 `alacritty_terminal`。[`iced_term` README](https://github.com/kemokempo/iced_term/blob/master/README.md) [`iced_term` Cargo.toml](https://github.com/kemokempo/iced_term/blob/master/Cargo.toml)
2. **对照验证：egui + `egui_term`**。不必一开始就从零写渲染器。`egui_term` 已经提供 Alacritty 网格到 egui 的渲染和基本交互，但仍是 0.1.0，README 明确说明功能不完整、仍在开发，当前功能表也没有列出终端鼠标模式。[`egui_term` README](https://github.com/kemokempo/egui_term/blob/main/README.md) [`egui_term` Cargo.toml](https://github.com/kemokempo/egui_term/blob/main/Cargo.toml)

这两个控件都不能直接视为 xterm.js 的等价替换。它们目前围绕本地 PTY 构造后端：`iced_term::Terminal::new` 会创建自己的 backend，公开命令把输入写回该 PTY；RShell 的数据源则是现有 SSH channel。验证时必须增加“外部字节流注入 + 输入字节回调”适配，确认可以把终端状态机与本地 PTY 解耦。[`iced_term` terminal.rs](https://github.com/kemokempo/iced_term/blob/master/src/terminal.rs) [`iced_term` backend.rs](https://github.com/kemokempo/iced_term/blob/master/src/backend.rs)

Zed、Lapce、COSMIC Terminal 和 WezTerm 证明了 Rust 原生终端可以达到生产级，但其终端 UI 都不是现成的通用控件。它们适合作为设计参考或二次移植来源，不适合作为 RShell 第一轮验证的直接依赖。

## 候选对比

| 路线 | 可复用终端能力 | 跨平台 | 许可证与 RShell 兼容性 | 接入现有 Rust core | 主要风险 | 判断 |
|---|---|---|---|---|---|---|
| **Iced + `iced_term`** | 独立 widget；`alacritty_terminal`；多实例、鼠标模式、选区、滚动、resize、超链接 | Iced 支持 Windows/macOS/Linux/Web；控件声明测试过三大桌面平台 | Iced、`iced_term` 为 MIT；Alacritty core 为 Apache-2.0，适合 RShell 的 Apache-2.0 | 中等。Iced 的消息/Subscription 模型和 RShell 已有 Tokio runtime 较契合；需拆开控件内置 PTY，接 SSH 字节流 | 控件作者明确标注 API 不稳定、功能不完整；小型社区维护；IME、bracketed paste、OSC 52、搜索等需实测 | **第一推荐，先验证** |
| **egui/eframe + `egui_term`** | 独立 widget；`alacritty_terminal`；基本输入、选区、滚动、resize、多实例、超链接 | eframe 官方支持 Windows/macOS/Linux/Android/Web；控件声明测试过三大桌面平台 | egui 为 MIT/Apache-2.0，控件 MIT，Alacritty core Apache-2.0 | 中等。立即模式容易嵌入现有状态；同样需把本地 PTY 改成 SSH 字节流接口 | 控件仅 0.1.0，功能声明比 `iced_term` 少；egui 官方也说明接口仍会破坏性变化；当前控件要求 Rust 1.92，而 RShell manifest 声明 1.90 | **保留为对照路线** |
| **Floem + 移植 Lapce 终端** | Lapce 已有生产使用的内置终端，底层 `alacritty_terminal`；渲染、事件、状态代码都存在 | Floem 官方声明支持 Windows/macOS/Linux | Floem MIT，Lapce Apache-2.0，与 RShell 兼容 | 中高。终端散落于 `lapce-app/src/terminal/*`，依赖 Lapce 配置、主题、proxy、RPC，不是独立 crate | 需要持续维护移植分支；Floem 仍在走向 v1，官方提示会有破坏性变化；当前 Floem 要求 Rust 1.91 并依赖其 winit fork | **适合第二阶段备选，不宜先做** |
| **GPUI + 借鉴 Zed 终端** | Zed 的终端和视图长期用于真实编辑器，底层也是 `alacritty_terminal` | GPUI 官方说明支持 macOS、Linux/FreeBSD 和 Windows | GPUI crate 为 Apache-2.0；但 Zed 的 `terminal`、`terminal_view` 两个 crate 明确为 GPL-3.0-or-later | 高。Zed 终端视图依赖 GPUI 以及 Zed 的 editor、settings、theme、project、workspace 等大量内部 crate | 不能把 GPL 终端代码直接移入当前 Apache-2.0 RShell 而保持现有许可；GPUI pre-1.0 且官方称频繁破坏性变化 | **可参考，不能作为直接复用方案** |
| **libcosmic + COSMIC Terminal** | 完整终端应用；`alacritty_terminal`；用 cosmic-text 自定义渲染，支持双向文本和连字 | libcosmic 基于 Iced/winit，但项目定位和依赖明显偏 COSMIC/Linux；不能据现有资料确认 cosmic-term 在 Windows/macOS 的产品级支持 | libcosmic MPL-2.0；cosmic-term GPL-3.0-only | 高。终端是应用源码，不是独立 widget；大量 COSMIC 配置、文件、Secret Service 和桌面集成 | GPL 代码复用限制；Linux/COSMIC 假设；当前 libcosmic/cosmic-term 要求 Rust 1.93 | **不建议用于 RShell 主线** |
| **WezTerm core + 任一 GUI** | 候选中 VT 能力最深：`wezterm-term`、termwiz、surface、bidi、图片协议等；完整 WezTerm 已在多平台验证 | WezTerm 是跨平台终端；termwiz 有 Unix/Windows 实现 | MIT，与 RShell 兼容 | 很高。完整 GUI crate 不发布；终端 core 位于大型 monorepo，依赖多组内部 workspace crate | `wezterm-term` 目前不在 crates.io，官方仍有开放的发布请求；使用 git/源码 vendoring 会承受内部 API、构建体积和同步成本 | **只在 Alacritty core 明确不够用时再评估** |

## 各路线的证据与含义

### 1. Iced + `iced_term`

Iced 本身是活跃维护的跨平台 Rust GUI，官方列出 Windows、macOS、Linux 和 Web，内置异步 action、可自定义 widget、wgpu 与软件渲染后端。[Iced 官方仓库](https://github.com/iced-rs/iced)

`iced_term` 比其他独立终端 widget 更接近可验证状态：

- 当前 manifest 对齐 Iced 0.14、`alacritty_terminal` 0.25.1 和 Tokio，并以 MIT 发布。[Cargo.toml](https://github.com/kemokempo/iced_term/blob/master/Cargo.toml)
- README 给出了完整的应用接线方式，包括 `Terminal` 状态、命令、事件、Subscription 和多个示例；功能表包含终端鼠标模式，仓库还提供 split view 与 focus 示例。[README](https://github.com/kemokempo/iced_term/blob/master/README.md)
- 其限制同样由官方 README 明说：Iced 尚未到 1.0，widget API 不保证稳定，终端功能也没有覆盖完毕。[README 的稳定性说明](https://github.com/kemokempo/iced_term/blob/master/README.md#unstable-widget-api)

对 RShell 的关键改造不是 UI，而是数据通道。现有控件后端会用 Alacritty 的 `tty::new` 创建本地 PTY，并持有 notifier；需要增加一种不创建 PTY的 backend，让 SSH 输出直接喂给 `Term`，键盘和鼠标编码后的字节则交给 `rshell-core` 的 SSH channel。[backend.rs](https://github.com/kemokempo/iced_term/blob/master/src/backend.rs)

### 2. egui/eframe + `egui_term`

egui/eframe 本身仍是成熟度较高、接入简单的 GUI 选择。官方说明 eframe 支持 Web、Linux、macOS、Windows、Android，也明确说明 egui 仍在快速演进、升级会有破坏性变化。[egui 官方仓库](https://github.com/emilk/egui)

先前方案假定需要从 `alacritty_terminal` 开始自绘；现在已有更低成本的验证入口：`egui_term` 直接提供 PTY 渲染、多实例、输入、滚动、选区、字体/配色和超链接，并声明测试过三个桌面系统。[`egui_term` README](https://github.com/kemokempo/egui_term/blob/main/README.md)

不过它的成熟度低于 `iced_term`：版本仍为 0.1.0，README 明确警告功能不完整；当前 manifest 跟随 egui/eframe 0.35、`alacritty_terminal` 0.26，并要求 Rust 1.92。[`egui_term` Cargo.toml](https://github.com/kemokempo/egui_term/blob/main/Cargo.toml)

因此 egui 路线应改为“先复用 `egui_term`，发现缺口后再局部改造”，而不是先投入完整自绘。

### 3. Floem + Lapce

Floem 是 Lapce 团队的原生 Rust GUI。官方声明支持 Windows、macOS、Linux，并提供 wgpu、Skia/Vello/Vger 以及 tiny-skia fallback；同时明确写明项目仍在成熟、到 v1 前会有缺失功能和破坏性变化。[Floem README](https://github.com/lapce/floem/blob/main/README.md)

Lapce 是最有价值的可移植参考：它是 Apache-2.0 项目，在三个桌面平台提供发行版，并已有内置终端。[Lapce README](https://github.com/lapce/lapce) 它的 app manifest 同时依赖 Floem 和 `alacritty_terminal`，[lapce-app Cargo.toml](https://github.com/lapce/lapce/blob/master/lapce-app/Cargo.toml)；终端视图直接实现 Floem `View`，可见于 [`terminal/view.rs`](https://github.com/lapce/lapce/blob/master/lapce-app/src/terminal/view.rs)。

问题在于这不是可发布的独立终端 widget。相关代码分布在 raw terminal、view、data、panel、tab 以及 proxy/RPC 层，抽取时要切断 Lapce 的主题、配置、编辑器和远端 proxy 依赖。它可能最终比小型第三方 widget 更可靠，但第一轮验证成本更高。

### 4. GPUI + Zed

GPUI 已作为 Apache-2.0 crate 发布，当前官方 manifest 为 0.2.2；官方文档覆盖 macOS Metal、Linux/FreeBSD Wayland/X11 和 Windows Win32/DirectWrite。[GPUI Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/gpui/Cargo.toml) [GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)

Zed 的终端是强有力的生产验证，也使用 `alacritty_terminal`。但它并不是 GPUI 自带 widget：Zed 的 [`terminal`](https://github.com/zed-industries/zed/blob/main/crates/terminal/Cargo.toml) 和 [`terminal_view`](https://github.com/zed-industries/zed/blob/main/crates/terminal_view/Cargo.toml) 都明确标为 GPL-3.0-or-later，terminal_view 还直接依赖 editor、project、settings、theme、workspace 等 Zed 内部 crate。

结论是可以学习其渲染、选择、输入和异步组织方式，但不能把这两个 crate 当成 Apache-2.0 RShell 的现成组件。

### 5. libcosmic + cosmic-term

COSMIC Terminal 是完整、活跃的 Rust 终端，官方说明使用 `alacritty_terminal`，以 cosmic-text 自绘，并支持双向文本和连字；默认使用 wgpu，失败时可回退 softbuffer/tiny-skia。[cosmic-term 官方仓库](https://github.com/pop-os/cosmic-term)

它的代码并不适合作为 RShell 通用组件：cosmic-term 是 GPL-3.0-only 应用，[Cargo.toml](https://github.com/pop-os/cosmic-term/blob/master/Cargo.toml)；libcosmic 是面向 COSMIC app/applet 的 MPL-2.0 工具包，虽然 `winit` feature 提供跨平台/X11 窗口路径，但项目文档、系统依赖和集成重心都在 COSMIC/Linux。[libcosmic 官方仓库](https://github.com/pop-os/libcosmic)

### 6. WezTerm 的可复用边界

WezTerm 本身是 MIT 的跨平台 GPU 终端和 multiplexer，是功能完整度的重要参照。[WezTerm README](https://github.com/wezterm/wezterm/blob/main/README.md) [`LICENSE.md`](https://github.com/wezterm/wezterm/blob/main/LICENSE.md)

它确实包含一个描述为“Virtual Terminal Emulator core”的 `wezterm-term` crate，依赖 bidi、cell、escape parser、surface、termwiz，并包含图片能力。[`term/Cargo.toml`](https://github.com/wezterm/wezterm/blob/main/term/Cargo.toml) 但这个 crate 仍未成为稳定的 crates.io 依赖；官方发布请求仍处于开放状态，也明确记录了外部使用者只能依赖 git/源码的问题。[发布 `wezterm-term` 的官方 issue](https://github.com/wezterm/wezterm/issues/6663)

`termwiz` 是已发布的 MIT crate，覆盖 Unix/Windows 终端 I/O、输入、surface 和 escape parser，[`termwiz/Cargo.toml`](https://github.com/wezterm/wezterm/blob/main/termwiz/Cargo.toml)；但 WezTerm 完整 GUI 仍是 `publish = false` 的大型应用 crate，并依赖 mux、Lua、字体、窗口和大量内部模块。[`wezterm-gui/Cargo.toml`](https://github.com/wezterm/wezterm/blob/main/wezterm-gui/Cargo.toml) 所以 WezTerm 不能直接提供一个可嵌入到 RShell 的 GUI terminal widget。

## 推荐验证矩阵

第一轮只做两个小型 prototype，共用同一组固定输入和验收脚本：

| 验证项 | Iced + `iced_term` | egui + `egui_term` |
|---|---:|---:|
| 不启动本地 PTY，注入模拟 SSH 字节流 | 必须 | 必须 |
| 输入、功能键、组合键输出为字节回调 | 必须 | 必须 |
| ANSI 16/256/TrueColor、清屏、光标、alternate screen | 必须 | 必须 |
| 中文/emoji/宽字符、连续 resize、历史滚动 | 必须 | 必须 |
| 拖选、复制粘贴、链接、搜索 | 必须 | 必须 |
| application mouse mode、bracketed paste | 必须 | 必须 |
| IME，尤其 Windows 中文输入和 macOS 输入法 | 必须 | 必须 |
| 高频输出下 UI 响应和内存增长 | 必须 | 必须 |
| 多终端标签，后台终端继续收流 | 必须 | 必须 |

决策规则：

- 若 `iced_term` 能在小幅 patch 内完成外部字节流接入，并通过输入法、鼠标模式和多标签验证，优先选择 Iced 路线。
- 若两者功能接近，按 RShell 整体 UI 的实现效率和视觉可控性选择；不要只依据单个终端 demo。
- 若两个独立 widget 都在 IME、mouse reporting、bracketed paste 或高频输出上出现结构性问题，再进入 Floem/Lapce 抽取验证。
- 只有在确认 `alacritty_terminal` 的 VT 能力本身不满足产品需求时，才投入 WezTerm core 的 vendoring/适配评估。

## 对原方案的具体调整

原来的“`alacritty_terminal` + 自定义 egui 渲染”仍然可行，但不再是最低成本的第一步。建议改为：

1. 先用 `iced_term` 完成 SSH 字节流解耦 prototype。
2. 并行用 `egui_term` 跑同一验证矩阵，复用现有设计文档中的 egui 窗口与状态面板思路。
3. 两个 prototype 都只放实验代码，不接生产启动路径。
4. 根据实际缺口选择维护一个小型 fork，或再考虑从 Lapce 抽取；暂不采用 Zed/COSMIC 的 GPL 终端代码。

这个调整能更直接回答真正的风险：不是 Rust 能否画终端，而是现成 Rust widget 能否稳定承载 RShell 的远程 SSH 字节流、输入法和跨平台交互。
