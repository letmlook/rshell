# 当前功能与限制

更新：2026-09-26。本文件替代历史未完成列表，区分实现范围与验收证据。

## 已接通的能力

- SSH/Telnet/Serial 会话保存、恢复、连接、断开和终端 I/O。
- 终端搜索、清屏、尺寸同步、发送与状态更新。
- SFTP 远程浏览、本地目录授权、单文件上传下载、新建目录、普通文件删除和真实队列。
- Local/Dynamic SSH 隧道，旧 Remote 规则兼容拒绝。
- 主机密钥确认与永久保存、SSH 密钥管理接口。
- 快速命令、触发器创建、删除、执行/开关与持久化。
- Rhai 宿主发送、列会话、执行快速命令接口。
- 主题、终端配色与事件订阅释放。
- 本地 WASM 插件发现、清单解析、编译加载与卸载。

## 核心接口与界面边界

触发器界面提供正则匹配后发送文本；通知、断开和日志动作由核心 API 提供。Rhai 没有完整脚本编辑器。WASM 加载不等于扩展点或宿主权限已执行，没有虚假的启用/禁用入口。

SSH 密码和密钥口令存放在 macOS 钥匙串（Keychain），会话 TOML 只保存凭据存在状态等元数据。读取旧版会话时会将其中的明文密码/口令迁移到钥匙串，并重写会话文件以移除秘密。迁移失败保留原文件并在会话列表显示持久加载问题，修复访问权限或配置后可点击“重试加载”。钥匙串条目缺失时连接失败关闭；用户可在会话列表右键选择“更新凭据”，重新输入密码或私钥口令，点击“保存凭据”后重新连接，不会从旧文件回退读取。此入口仅接收新凭据，不显示已存储的秘密。钥匙串不可访问时须先修复 macOS 钥匙串访问权限。macOS 可能在钥匙串访问时显示系统授权提示。主密码仅用于应用锁定/验证，不是凭据保险库，也不加密钥匙串。钥匙串系统提示、旧版数据迁移及缺失凭据恢复尚无 macOS 人工验收记录。主题选择和传输队列不承诺跨进程恢复。触发匹配以 UTF-8 输出片段为单位，不保证跨分片模式匹配。

传输队列界面提供可见的「暂停/继续」操作，调用已注册的 `PauseTransfer`/`ResumeTransfer` 命令；取消只有 `CancelTransfer` 命令、尚无界面入口。三者通过按任务的 `watch` 控制通道驱动 SFTP 拷贝循环，在每个分块前检查：暂停后字节停止增长、恢复后从已传字节继续；取消立即中止并保持 `cancelled` 终态，不会被改写为完成或失败。该行为由协议层拷贝循环与任务服务的自动化测试覆盖，真实 SSH/SFTP 服务器上的暂停/恢复观感仍属待验收项。

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
