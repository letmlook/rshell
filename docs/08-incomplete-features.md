# 当前功能与限制

更新：2026-10-03（第二轮问题修复轮）。本文件替代历史未完成列表，区分实现范围与验收证据。

## 已接通的能力

- SSH/Telnet/Serial 会话保存、恢复、连接、断开和终端 I/O。
- 终端搜索、清屏、尺寸同步、发送与状态更新。
- 终端面板短标签：会话名优先（超长按字符截断加省略号），无会话名时退回首 8 位会话 id。会话改名后重新激活会同步刷新标签。
- SFTP 远程浏览、本地目录授权、单文件上传下载、新建目录、普通文件删除和真实队列。远程/本地文件列表支持右键菜单：本地列表给「上传到远程」，远程列表给「下载到本地/新建文件夹/删除/刷新」；多选普通文件可一次批量入队（目录与不安全名称自动剔除），单条失败不阻断其余文件，失败条目汇总提示。
- Local/Dynamic SSH 隧道，旧 Remote 规则兼容拒绝。
- 主机密钥确认与永久保存、SSH 密钥管理接口（仅为托管存储，不参与会话认证，见下文边界说明）。
- 快速命令、触发器创建、删除、执行/开关与持久化。
- Rhai 宿主发送、列会话、执行快速命令接口。
- 主题、终端配色与事件订阅释放。
- 本地 WASM 插件发现、清单解析、编译加载与卸载。

## 核心接口与界面边界

触发器界面提供正则匹配后发送文本；通知、断开和日志动作由核心 API 提供。Rhai 没有完整脚本编辑器。WASM 加载不等于扩展点或宿主权限已执行，没有虚假的启用/禁用入口。

SSH 密码和密钥口令存放在 macOS 钥匙串（Keychain），会话 TOML 只保存凭据存在状态等元数据。读取旧版会话时会将其中的明文密码/口令迁移到钥匙串，并重写会话文件以移除秘密。迁移失败保留原文件并在会话列表显示持久加载问题，修复访问权限或配置后可点击“重试加载”。钥匙串条目缺失时连接失败关闭；用户可在会话列表右键选择“更新凭据”，重新输入密码或私钥口令，点击“保存凭据”后重新连接，不会从旧文件回退读取。此入口仅接收新凭据，不显示已存储的秘密。钥匙串不可访问时须先修复 macOS 钥匙串访问权限。macOS 可能在钥匙串访问时显示系统授权提示。主密码仅用于应用锁定/验证，不是凭据保险库，也不加密钥匙串。钥匙串系统提示、旧版数据迁移及缺失凭据恢复尚无 macOS 人工验收记录。主题选择和传输队列不承诺跨进程恢复。触发匹配以 UTF-8 输出片段为单位，不保证跨分片模式匹配。

SSH 密钥管理面板提供生成、导入、删除与列表展示，当前仅为托管存储：私钥保存在应用数据目录的 keys_dir（生成/导入时提供口令则以 OpenSSH 加密格式落盘），没有「关联到会话」入口，不参与会话认证。会话公钥认证使用会话自身凭据配置的外部私钥文件路径，口令经钥匙串提供。因此，在密钥管理面板生成或导入的私钥（含带口令加密的）目前无法在本应用内用于 SSH 连接；界面提示与该边界一致。

传输队列界面提供可见的「暂停/继续/取消/删除」操作，分别调用 `PauseTransfer`/`ResumeTransfer`/`CancelTransfer`/`RemoveTransfer`。前三者通过按任务的 `watch` 控制通道驱动 SFTP 拷贝循环，在每个分块前检查：暂停后字节停止增长、恢复后从已传字节继续；取消立即中止并保持 `cancelled` 终态，不会被改写为完成或失败。「删除」只把**终态**（完成/失败/已取消）条目移出队列列表，不删除本地或远端已传输的文件；活跃任务调用会返回 `InvalidState`，必须先取消。未知条目重复删除按幂等处理，不报错。面板标题栏另有队列管理菜单，可按「清除已完成/失败/已取消/全部已结束」批量移除，并显示各分组当前条数；批量逐条调用 `RemoveTransfer` 而非新增批量命令，以便部分失败可见（否则用户会以为已清空而实际仍有残留）。进行中的任务在任何分组里都不会被批量清除。该行为由协议层拷贝循环与任务服务的自动化测试覆盖，真实 SSH/SFTP 服务器上的暂停/恢复观感仍属待验收项。

远端文件面板默认打开登录用户的工作目录，而不是文件系统根 `/`：新增 `GetRemoteHomeDir` 命令，用 SFTP `realpath(".")` 解析（sftp 子系统启动时的当前目录即用户 home，远端无需额外支持 `~` 展开），解析失败回退 `/` 并在前端保持当前目录，不把远端面板打成空白。外部显式传入 `remotePath` 时不覆盖。

本地目录记忆上次打开位置：存 `localStorage`（纯 UI 偏好，无跨进程/跨设备一致性要求），仅接受绝对路径并归一化尾部分隔符，存储不可用时降级为「不记忆」且不抛错。记忆的目录若已失效，错误由面板错误行显示真实原因，用户可点「更换」重新选择，不静默清空。

### 本地文件系统权限（全局读写）

Tauri 的 fs scope 分「命令权限」与「路径 scope」两层。此前 capability 只声明了 `fs:allow-read-dir` / `fs:allow-stat` 且**没有任何路径 scope**，而 scope 是运行时状态：`tauri-plugin-dialog` 只在用户点选目录时自动 `allow_directory`，进程重启后即为空。后果是「上次打开的目录」从 localStorage 恢复时没有经过对话框，读取被拒（`forbidden path: ...`，该提示仅在 debug 构建出现，release 返回 `PathForbidden`）。

按产品决定改为**默认全局可读可写**：capability 声明 `fs:read-all` + `fs:write-all`，并加一条 `{ "identifier": "fs:scope", "allow": ["**"] }` 覆盖全部路径。`deny` 优先于 `allow`（tauri 先判 `is_forbidden`），因此同时保留 `fs:deny-webview-data-windows` / `-linux`，WebView 自身数据目录仍不可读写。

风险边界（如实记录）：授予的是应用自身 WebView 的权限，`fs:write-all` 包含 `mkdir`/`create`/`copy_file`/`remove`/`rename`/`truncate`/`write*`，即 WebView 可创建、覆盖、**删除和重命名**进程权限范围内的任意文件。本应用以自己的进程权限运行，该范围与 Xftp/WinSCP 等同类工具一致。`tests/unit/capabilityScope.spec.ts` 锁定三项：存在 `**` scope、读写权限未被收窄、WebView 数据目录仍在 deny 列表。

终端复制粘贴快捷键为 `Ctrl+Shift+C` / `Ctrl+Shift+V`（macOS 另接受 `Cmd`），经既有的 `rshell:terminal-action` 路由到当前激活终端。裸 `Ctrl+C`（SIGINT）与 `Ctrl+V`（quoted-insert）不劫持，原样透传给远端。无选中内容或剪贴板为空时给出可见提示，剪贴板不可用时提示失败原因，不静默吞掉。

SFTP 下载方向改为并发区间读取。根因是 `russh-sftp` 的 `File::poll_read` 只有一个在途 READ 槽，串行读取的吞吐上限为 `CHUNK_SIZE / RTT`；上传方向用 `write_nowait` 本就支持 `max_concurrent_writes` 路并发，因此下载长期明显慢于上传。现按 64 KiB 切分连续不重叠区间，最多 8 个 READ 同时在途，写入侧仍严格按 offset 升序串行，落盘内容与串行版本逐字节一致；小于 128 KiB 的文件退回串行以省掉多句柄开销。并发区间共用一个**远端句柄池**（整个传输只开 8 次句柄）——逐区间 `open` 会把 OPEN/CLOSE 放大到每 64 KiB 一对，1 GiB 文件就是 16384 次 OPEN + 16384 次 CLOSE，协议消息数是数据量的 3 倍；池空时退回现开一个，保证任务被中止丢句柄后读取不会永久阻塞。区间划分的连续/不重叠/完整覆盖由纯函数 `plan_ranges` 的测试锁定，句柄池的借还不泄漏/不超发/并发互异另有测试，错序完成、进度单调性、取消中止与源变短报错均有测试覆盖。**真实服务器上的实际吞吐提升倍数未验证**，局域网与跨网的对比数据待补。

### 传输吞吐

实测局域网下曾只有约 2 MB/s，而同链路的 Xshell 可达约 50 MB/s。已定位并修复三处：

1. **SSH 传输 socket 未关闭 Nagle（主因）**。`russh::client::connect` 内部直接 `TcpStream::connect`，从不设 `TCP_NODELAY`，且 `russh::client::Config` 没有对应开关。Nagle 把小写入攒起来等对端确认，而对端 TCP 的延迟 ACK 又在攒数据等满一个包，两者叠加出数百毫秒级停顿。SFTP 是高频小包协议，正好踩中。现由 `open_nodelay_socket` 自行建连并关闭 Nagle，再走 `russh::client::connect_stream`；测试对真实 socket 断言 `nodelay()` 为真，防止改回 russh 的封装。
2. **下载只有 1 个在途 READ**（见上）。改为最多 8 路并发区间读取。
3. **逐区间开关句柄**（见上）。改为远端句柄池，整个传输只开 8 次。

**修复后的实际吞吐倍数仍未验证**，需在真实服务器上与 Xshell 对比后再据实记录；本节不把 50 MB/s 标为已达成。

## 不在本轮范围

RDP 已删除。Remote Forward、FTP/FTPS、Windows ConPTY、截图、录屏、目录递归传输、完整插件生态和新协议均不是本轮新增目标。没有后端支撑的入口被移除，不以按钮占位冒充实现。

## macOS 发布预检与签名/公证

桌面构建入口仍为 `npm run tauri:build`，产物 Bundle ID 为 `com.letmlook.rshell`；本地与 CI 的预检/审计只走仓库脚本：

- `bash scripts/macos-release-preflight.sh --unsigned <app-path>`：不接触签名/公证凭据，验证 Bundle ID、Info.plist、目录布局。
- `bash scripts/macos-release-preflight.sh <app-path>`：要求设置 `APPLE_SIGNING_IDENTITY` 与 `APPLE_NOTARY_PROFILE`，执行 `codesign --verify --deep --strict`、`xcrun notarytool submit --wait`、`xcrun stapler staple`、`spctl --assess`。脚本不会回显任一凭据。
- `bash scripts/macos-verify-app.sh <app-path>`：校验已构建产物的 Bundle ID、严格代码签名与 Gatekeeper 评估。

在 Developer ID 证书、notarytool 凭据和真实公证步骤落地之前，CI 与本地文档只允许运行 `--unsigned` 预检与 `macos-verify-app.sh`，不允许把未签名/未公证的 `.app` 写为“已发布”。签名与公证的成功证据将单独记录到 [macOS 验证](09-macos-validation.md)。

## 尚待真实环境验收

自动检查、调试应用打包和基础 GUI 交互证据见 [macOS 验证](09-macos-validation.md)。未执行的 GUI 写操作、真实 SSH/SFTP 与隧道、物理串口、签名公证不能仅凭单元测试标记完成。

本轮实现加固与自动化回归已完成，但不是全部真实环境验收完毕。没有服务器/设备证据的条目继续标待验证，不通过改写文档消除验收缺口。
