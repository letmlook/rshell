# 贡献指南

> 感谢你考虑为 RShell 做出贡献！

本文档说明参与 RShell 项目的开发流程、代码规范与提交流程。**完整规范以 [`docs/05-development-standards.md`](docs/05-development-standards.md) 为准**，下面对常用条目做摘要。

---

## 1. 行为准则

请保持尊重与建设性的沟通。所有参与者都应遵守基本的开源社区礼仪。

---

## 2. 问题反馈

- **Bug 报告**：请使用 Issue 模板，提供最小复现步骤、操作系统、Rust 版本、相关日志（启用 `RUST_LOG=rshell=debug` 后重新运行）。
- **功能请求**：先在 Discussions 区讨论，确认符合项目方向后再创建 Issue。
- **安全问题**：**请勿**在公开 Issue 中披露安全漏洞，请私下联系维护者。

---

## 3. 开发环境搭建

需要 Rust **1.80 或更高**（`rust-toolchain.toml` 已锁定）：

```bash
# 克隆
git clone https://github.com/letmlook/rshell.git
cd rshell

# 安装组件
rustup component add rustfmt clippy

# 可选工具
cargo install cargo-watch cargo-deny cargo-nextest
```

### 平台依赖

| 平台 | 必需依赖 |
|------|----------|
| Windows | Visual Studio Build Tools（C++ 桌面开发）、Vulkan SDK、CMake |
| macOS | Xcode Command Line Tools、CMake、pkg-config（`brew install cmake pkg-config`） |
| Linux | build-essential、pkg-config、libssl-dev、libfontconfig1-dev、libfreetype6-dev、libexpat1-dev、cmake、libvulkan-dev、libx11-dev、libxkbcommon-dev、libwayland-dev |

详细说明见 [`docs/07-project-setup-guide.md`](docs/07-project-setup-guide.md) §3.3。

---

## 4. 工作流程

### 4.1 分支策略（Trunk-based + Feature Branches）

```
main (trunk)
├── feat/<描述>           ← 功能分支
├── fix/<描述>            ← 修复分支
├── refactor/<描述>       ← 重构分支
└── release/v<版本>       ← 发布分支（可选）
```

**规则**：
- 从 `main` 拉出新分支进行开发
- 完成功能后通过 PR 合并回 `main`（Squash merge）
- **禁止直接 push 到 `main`**
- 每个 PR 必须通过 CI + 至少 1 人 Code Review
- 分支命名全小写，单词间用 `-` 连接

### 4.2 提交流程

1. 从 `main` 拉出分支：`git checkout -b feat/<描述>`
2. 进行开发，每个提交只做一件事（原子提交）
3. 提交前运行：`cargo fmt --all && cargo clippy --workspace --all-targets && cargo test`
4. 推送分支并创建 PR
5. 根据 Code Review 反馈修改

### 4.3 提交规范（Conventional Commits）

```
<type>(<scope>): <subject>

[body]

[footer]
```

| Type | 说明 |
|------|------|
| `feat` | 新功能 |
| `fix` | 修复 Bug |
| `refactor` | 重构（不改变行为） |
| `test` | 测试相关 |
| `docs` | 文档 |
| `chore` | 构建 / 工具 |
| `perf` | 性能优化 |
| `style` | 格式（不影响逻辑） |

**Scope** 取模块名：`terminal` / `ssh` / `telnet` / `serial` / `rdp` / `sftp` / `transfer` / `session` / `ui` / `security` / `script` / `plugin` / `core` / `infra`。

**规则**：
- subject 不超过 72 字符，英文小写开头，不加句号
- 破坏性变更在 footer 标注 `BREAKING CHANGE:`

### 4.4 版本与 Tag

遵循 Semantic Versioning（`MAJOR.MINOR.PATCH`）。每个里程碑的 tag 计划见 [`docs/05-development-standards.md`](docs/05-development-standards.md) §3.3。

---

## 5. 代码规范要点

完整规范见 [`docs/05-development-standards.md`](docs/05-development-standards.md)。摘要：

### 5.1 命名

| 元素 | 风格 |
|------|------|
| 类型 / 结构体 / 枚举 | PascalCase |
| 函数 / 方法 | snake_case |
| 变量 / 字段 | snake_case |
| 常量 | SCREAMING_SNAKE_CASE |
| 模块 / Crate | kebab-case |

### 5.2 模块组织

```
rshell-<crate>/
└── src/
    ├── lib.rs          # crate 入口，仅声明模块与 re-export
    ├── <module>/
    │   ├── mod.rs      # 模块声明
    │   ├── service.rs  # 业务逻辑
    │   └── tests.rs    # 或 #[cfg(test)] mod tests
    └── error.rs
```

### 5.3 错误处理

- 用 `thiserror` 定义领域错误
- **禁止** `unwrap()` / `expect()`（测试代码除外）
- 跨 crate 错误转换用 `From` trait

### 5.4 异步

- I/O 密集用 `tokio::spawn`，CPU 密集用 `spawn_blocking`
- 使用 `tokio::select!` 处理多路复用与超时
- 后端 ↔ 前端通信使用 `tokio::sync::mpsc`，内部使用 `tokio::sync::broadcast`

### 5.5 日志

- 使用 `tracing` 结构化日志
- 关键函数添加 `#[instrument]`
- 级别：ERROR（不可恢复）/ WARN（可恢复）/ INFO（关键业务）/ DEBUG（开发）/ TRACE（极细粒度）

---

## 6. 架构约束（最高优先级）

RShell 的核心是**严格的前后端分离**（[`docs/05-development-standards.md`](docs/05-development-standards.md) §2）：

```
□ 后端代码（rshell-core / rshell-protocol / rshell-infra）中无 `use gpui::*`
□ 前端 View / ViewModel 无 `use rshell_core::*`（仅 main.rs / bridge.rs 可用）
□ 跨层通信仅通过 rshell-api 的 AppCommand / AppEvent
□ ViewModel 不持有任何 Service 引用
□ AppCommand：动词+名词，全部具名字段，实现 Serialize/Deserialize/Clone
□ AppEvent：过去时态，携带完整快照数据
```

任何破坏此约束的 PR 将被拒绝。

---

## 7. 代码审查清单

### 7.1 通用

```
□ cargo fmt 与 cargo clippy 通过？
□ 单元测试充分？
□ 错误处理完整（无 unwrap/expect）？
□ 日志级别合理？
□ 公开 API 有文档注释（///）？
```

### 7.2 前后端分离

```
□ 后端代码引用了 GPUI 类型？（禁止）
□ 前端 View / ViewModel 引用了 rshell-core？（禁止）
□ 跨层通信通过 Command/Event？
□ ViewModel 只包含 UI 状态 + 后端投影？
```

### 7.3 安全

```
□ 涉及密钥 / 密码的代码走 ring 加密？
□ 私钥 / 密码不写入日志？
□ 主机密钥验证逻辑完善？
```

---

## 8. 测试

- **单元测试**：与代码同模块的 `#[cfg(test)] mod tests`
- **集成测试**：放在各 crate 的 `tests/` 目录
- **基准测试**：使用 `criterion`（已在 workspace 中）
- 推荐使用 `cargo nextest run` 加速执行

---

## 9. 文档

- 修改代码时同步更新相关 `docs/` 文档
- 新增模块 / crate 时更新 `README.md` 中的「项目结构」表
- 新增功能时更新 `CHANGELOG.md`

---

## 10. 许可

提交 PR 即表示你同意以 [Apache License 2.0](LICENSE) 协议授权你的贡献。

感谢你的贡献！🎉