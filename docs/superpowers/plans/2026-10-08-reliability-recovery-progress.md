# RShell 可靠性优化实施进度

更新时间：2026-10-08 22:20（Asia/Shanghai）。状态：本轮 4 个任务实施完成；每任务独立审查为 `DONE_WITH_CONCERNS`，全部修复已落地；集成验证在 Windows 环境完成并记录到 `docs/09-macos-validation.md`。分支未合并、推送、发布；macOS 产品验收按 spec 划分仍待真机。

## 保存位置

- 分支：`codex/reliability-recovery`。
- 托管工作区：`C:/Users/lp/.codex/worktrees/reliability-recovery/rshell`。
- 原始工作区：`C:/code/github/rshell`，仍在 main；未合并、推送或发布。
- 设计及实施计划提交：`b4e3d687c1e45a215891437df9a63b9c3b296600`。
- 本轮 HEAD：`6445486`（Task 3 审查修复）。基线 HEAD：Task 1 实现 `c1afefc`，本轮累计新增：
  - `86c6ac0` Task 1 审查修复（IpcError mapping 加固）
  - `07f7c69` Task 2 实施（staged SFTP commit）
  - `28ba8df` Task 2 审查修复（commit_strategy UI 渲染 + ipcContract 字段对账）
  - `3c309f6` Task 3 实施（凭据分类 + host-key 生命周期）
  - `6445486` Task 3 审查修复（双击取消幂等）

## 已完成

Task 1 的实现、任务范围内回归和自查已完成并提交；独立审查未完成，不能据此认定整个任务验收完成。

- SSH 请求的 10 秒预算覆盖客户端锁等待、入队与执行，过期或调用方已放弃的请求不会开始执行。
- 每个终端可独立取消；输出接收与写入分离，拥塞时关闭不再排在普通请求后面。客户端销毁清理旧终端及输出路由。
- 后端提供 `terminal_recovery_required` 错误，界面在首次恢复错误时持续提示结果不确定，限制输入、退格、粘贴和 resize。
- 手动恢复执行真实断开、连接、终端创建与附加；不重放命令，旧异步结果按代际隔离。连接事件先到时也须等终端附加成功才开放输入。
- 主终端故障沿用整条连接清理契约；附加终端故障可隔离；界面跟随后端真实断开状态。
- 当前功能文档已同步上述行为与等待预算。

## 实际验证

| 检查 | 最近结果 |
|---|---|
| 协议层完整库测试 | 69 通过，0 失败 |
| 核心层完整库测试 | 189 通过，0 失败 |
| 前端完整测试 | 37 文件，279 通过，3 跳过 |
| 前端类型检查 | 退出 0 |
| 协议及核心层 clippy，所有 target，`-D warnings` | 退出 0 |
| rustfmt 与改动空白检查 | 已执行，通过 |

已观察失败后修复的场景包含：饱和队列、无界主终端应答、关闭排队、过期输入迟到执行、客户端销毁未取消终端、核心锁等待、首次超时界面恢复及终端附加未完成即开放输入。

完整命令、失败与通过计数、实现说明见 [Task 1 实现报告](../../research/2026-10-08-ssh-recovery-implementation-report.md)。

## 暂停点与尚未通过的门槛

- Task 1 独立审查正在进行时被用户暂停，审查代理已停止，尚无最终审查结论。恢复时重新审查 `b4e3d68..c1afefc` 的代码变化。
- Tauri 新增错误映射测试尚未运行，需在最终 `cargo test --workspace` 中执行；不能把协议与核心测试结果等同于整个 Rust workspace 通过。
- 关闭标签若在获取客户端锁前超时，会报告失败，远端资源仍依赖后续整条连接清理；该边界需由审查评估，不隐藏为清理成功。
- 整体 shared verification 尚未通过。脚本基线在 Windows/WSL 与 Git Bash 下均有路径或夹具预期问题。
- npm audit 基线为 0 漏洞。Rust audit 使用独立数据库，沿用 RSA 例外后退出 0，但仍有 9 项非阻断警告；尚未完成修改后的最终审计与完整构建。
- macOS 的真实 SSH/SFTP、钥匙串及恢复操作均待实机验收。

## 后续任务

1. 先完成 Task 1 独立审查。重大问题须补失败回归后修复，再审查；不要重新实施已提交改动。
2. Task 2 尚未开始：临时 SFTP 文件、最终提交与清理、取消提交竞争、失败与取消任务从零重试。
3. Task 3 尚未开始：凭据错误分类与恢复入口、并发主机密钥确认队列、取消与过期事件同步。
4. Task 4 尚未完成：跨任务契约检查、完整质量门禁、全分支审查、验证记录与 macOS 产品验收。

## 下一阶段已调查事实

- russh-sftp 2.4.0 的高层 `SftpSession` 没有 posix-rename 入口，但这不代表服务器不支持扩展。可研究额外 SFTP 通道上的 `RawSftpSession` 能力协商和安全提交，保留高层接口处理拷贝。不能仅因库入口缺失就禁用所有服务器的安全覆盖。
- 无法安全提交时保持旧目标并报告失败，禁止先删旧目标再 rename。独占临时创建、提交前冲突策略、权限失败与“不存在”的区别均需测试。
- 当前取消任务会在 worker 清理前发布终态；新提交逻辑必须裁决取消与提交竞争，不能出现已取消却覆盖目标。
- `App.vue` 持有传输面板控制与完整任务快照，需要参与重试与结果展示。
- 凭据存储已能区分缺失与后端错误，但 repository/service 合并原因；应保留类型化、脱敏的分类，并保持 Task 1 新增错误类型。
- 主机密钥请求的生命周期须涵盖 SSH 检查 future 被取消、后端超时、并发提交及永久信任保存跨 await 的竞争，避免复活过期请求或清除另一个请求。

## 恢复入口

读取 [完整方案](../specs/2026-10-08-reliability-improvement-design.md)、[实施计划](2026-10-08-reliability-recovery.md)、本文件和 [Task 1 实现报告](../../research/2026-10-08-ssh-recovery-implementation-report.md)，核对分支及提交后从 Task 1 审查继续。

本地 `.superpowers/sdd/2026-10-08-reliability-recovery/` 的 brief、report、diff package 和 advisory 数据库仍保留，但它们不是唯一恢复依据。已将执行台账保存为 [项目内台账](2026-10-08-reliability-recovery-execution-ledger.md)，避免依赖未跟踪目录或聊天历史。
