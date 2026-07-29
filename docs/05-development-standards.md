# RShell 开发规范文档

> **文档版本**：v1.0  
> **编写日期**：2026-07-29  
> **项目名称**：RShell  
> **关联文档**：`docs/03-detailed-design.md`

---

## 1. Rust 编码规范

### 1.1 命名约定

| 元素 | 风格 | 示例 |
|------|------|------|
| 类型/结构体/枚举 | PascalCase | `TerminalBuffer`, `SessionConfig` |
| 函数/方法 | snake_case | `send_input()`, `get_buffer_snapshot()` |
| 变量/字段 | snake_case | `session_id`, `buffer_size` |
| 常量 | SCREAMING_SNAKE_CASE | `MAX_BUFFER_SIZE`, `DEFAULT_PORT` |
| 模块 | snake_case | `core_terminal`, `protocol_ssh` |
| Crate | kebab-case | `rshell-core`, `rshell-api` |
| Trait | PascalCase | `Connection`, `TerminalService` |
| 宏 | snake_case + `!` | `rshell_plugin_export!()` |
| 类型参数 | 单个大写字母或描述性 PascalCase | `T`, `E`, `Config` |
| 生命周期 | 小写字母 + `'` | `'a`, `'ctx` |

### 1.2 模块组织

```
rshell-core/
├── src/
│   ├── lib.rs              # crate 入口，公开 API re-export
│   ├── terminal/           # 终端模块
│   │   ├── mod.rs          # 模块声明 + 公开接口
│   │   ├── service.rs      # TerminalService 实现
│   │   ├── buffer.rs       # 缓冲区管理
│   │   ├── pty.rs          # PTY 抽象层
│   │   └── tests.rs        # 模块内测试
│   ├── session/            # 会话模块
│   │   ├── mod.rs
│   │   ├── service.rs
│   │   └── repository.rs
│   └── error.rs            # crate 级错误定义
```

**规则**：
- 每个模块 `mod.rs` 只声明子模块和公开 re-export
- 私有实现细节放在子模块中
- 测试代码放在同模块的 `tests.rs` 或 `#[cfg(test)] mod tests`
- 错误类型在模块内定义，crate 级错误在 `error.rs` 汇总

### 1.3 错误处理

```rust
// 规则 1：使用 thiserror 定义领域错误
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("Terminal not found: {0}")]
    NotFound(Uuid),
    #[error("PTY creation failed: {0}")]
    PtyError(#[from] PtyError),
    #[error("Connection closed")]
    ConnectionClosed,
}

// 规则 2：后端 Service 返回 Result<T, ServiceError>
pub type ServiceResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// 规则 3：禁止 unwrap()/expect()（测试代码除外）
// BAD:
let session = sessions.get(&id).unwrap();
// GOOD:
let session = sessions.get(&id).ok_or(SessionError::NotFound(id))?;

// 规则 4：跨 crate 错误转换使用 From trait
impl From<russh::Error> for ProtocolError { ... }
```

### 1.4 异步规范

```rust
// 规则 1：异步函数使用 async fn，不返回 BoxFuture（除非 trait 需要）
async fn connect(&self, config: &SessionConfig) -> Result<ConnectionHandle>;

// 规则 2：I/O 密集用 tokio::spawn，CPU 密集用 spawn_blocking
tokio::spawn(async move { /* I/O */ });
tokio::task::spawn_blocking(move || { /* CPU */ });

// 规则 3：使用 tokio::select! 处理多路复用，设置超时
tokio::select! {
    result = session.read() => { /* ... */ }
    _ = tokio::time::sleep(Duration::from_secs(30)) => {
        // 超时处理
    }
}

// 规则 4：Channel 通信优先于共享锁
// 后端 → 前端通信使用 tokio::sync::broadcast
// 命令队列使用 tokio::sync::mpsc
```

### 1.5 日志规范

```rust
use tracing::{debug, info, warn, error, instrument};

// 规则 1：使用 tracing 结构化日志
info!(session_id = %id, host = %config.host, "Session connected");
error!(error = %e, terminal_id = %id, "Terminal output failed");

// 规则 2：关键函数使用 #[instrument]
#[instrument(skip(self), fields(terminal_id = %id))]
async fn send_input(&self, id: Uuid, data: &[u8]) -> Result<()> { ... }

// 规则 3：日志级别
// ERROR: 不可恢复错误（连接断开、文件损坏）
// WARN:  可恢复异常（重连成功、降级处理）
// INFO:  关键业务事件（连接/断开、传输完成）
// DEBUG: 开发调试信息（VT 序列、缓冲区变化）
// TRACE: 极细粒度（每个字节、每帧渲染）
```

---

## 2. 前后端分离开发规范

### 2.1 分层规则

| 规则 | 说明 |
|------|------|
| 后端不引用 GPUI | `rshell-core` / `rshell-protocol` / `rshell-infra` 中不可出现 `use gpui::*` |
| 前端不直接调用后端 | `rshell-ui` 通过 `rshell-api` 的 Command/Event 与后端通信 |
| API 层零运行时依赖 | `rshell-api` 仅依赖纯数据类型 crate（serde/uuid），不依赖运行时 crate（tokio/gpui） |
| 数据跨边界用快照 | 后端向前端传递 `Snapshot` 结构体，不传递引用/句柄 |

### 2.2 Command 定义规范

```rust
// 位置：rshell-api/src/commands.rs
// 命名：动词 + 名词，表示用户意图
pub enum AppCommand {
    // 会话相关
    ConnectSession { session_id: Uuid },
    DisconnectSession { session_id: Uuid },
    CreateSession { config: SessionConfig },
    
    // 终端相关
    SendInput { terminal_id: Uuid, data: Vec<u8> },
    ResizeTerminal { terminal_id: Uuid, rows: u16, cols: u16 },
    
    // 文件传输
    EnqueueUpload { transfer_id: Uuid, items: Vec<TransferItem> },
    CancelTransfer { transfer_id: Uuid },
}

// 规则：
// 1. Command 是不可变意图，不包含回调
// 2. 字段全部具名，不使用元组变体
// 3. 所有字段类型必须实现 Serialize + Deserialize + Clone
```

### 2.3 Event 定义规范

```rust
// 位置：rshell-api/src/events.rs
// 命名：过去时态，表示已发生的事实
pub enum AppEvent {
    // 连接状态变化
    ConnectionStateChanged {
        session_id: Uuid,
        state: ConnectionState,
    },
    // 终端输出
    TerminalOutput {
        terminal_id: Uuid,
        snapshot: TerminalBufferSnapshot,
    },
    // 传输进度
    TransferProgress {
        transfer_id: Uuid,
        bytes_transferred: u64,
        bytes_total: u64,
    },
}

// 规则：
// 1. Event 是只读快照，不引用可变状态
// 2. 携带完整数据（前端无需再查询）
// 3. 大数据用 Snapshot 结构体，不用 Vec<u8> 裸传
```

### 2.4 ViewModel 编写规范

```rust
// 位置：rshell-ui/src/view_models/
// 命名：XxxViewModel，表示某个 UI 区域的状态投影

pub struct TerminalViewModel {
    // 后端状态投影
    pub buffer: TerminalBufferSnapshot,
    pub connection_state: ConnectionState,
    pub title: String,
    
    // 本地 UI 状态（后端不感知）
    pub scroll_offset: usize,
    pub is_search_mode: bool,
    pub search_query: String,
    pub selection: Option<Selection>,
}

// 规则：
// 1. ViewModel 只包含 UI 渲染所需数据 + 本地交互状态
// 2. 后端状态字段与后端 Event 同步更新
// 3. 本地 UI 状态（滚动位置、搜索框）仅前端管理
// 4. ViewModel 不持有任何 Service 引用
```

### 2.5 数据流检查清单

每个新功能开发时，验证数据流是否符合规范：

```
□ 用户操作 → View 发出 Command（不是直接调用 Service）
□ Command → CommandDispatcher → 后端 Service 处理
□ 后端 Service → 发出 Event
□ Event → EventBus → ViewModel 更新
□ ViewModel 变化 → View 自动重绘
□ 后端代码中无 use gpui::*
□ 前端代码中无 use rshell_core::*
```

---

## 3. Git 分支策略与提交规范

### 3.1 分支策略（Trunk-based + Feature Branches）

```
main (trunk)
├── feat/ssh-password-auth     ← 功能分支
├── feat/sftp-file-browser     ← 功能分支
├── fix/terminal-resize-crash  ← 修复分支
├── refactor/event-bus         ← 重构分支
└── release/v0.1.0             ← 发布分支（可选）
```

| 分支类型 | 命名 | 生命周期 | 合并方式 |
|----------|------|----------|----------|
| main | `main` | 永久 | — |
| 功能分支 | `feat/<描述>` | 功能完成后合并 | Squash merge |
| 修复分支 | `fix/<描述>` | 修复完成后合并 | Squash merge |
| 重构分支 | `refactor/<描述>` | 重构完成后合并 | Squash merge |
| 发布分支 | `release/v<版本>` | 发布完成后合并 | Merge |

**规则**：
- 功能分支从 `main` 拉出，完成后通过 PR 合并回 `main`
- 每个 PR 必须通过 CI + 至少 1 人 Code Review
- 禁止直接 push 到 `main`
- 分支命名全小写，单词间用 `-` 连接

### 3.2 提交规范（Conventional Commits）

```
<type>(<scope>): <subject>

[body]

[footer]
```

**Type 列表**：

| Type | 说明 | 示例 |
|------|------|------|
| `feat` | 新功能 | `feat(terminal): add 256 color support` |
| `fix` | 修复 Bug | `fix(ssh): handle connection timeout correctly` |
| `refactor` | 重构（不改变行为） | `refactor(event-bus): simplify dispatcher` |
| `test` | 测试相关 | `test(sftp): add upload resume tests` |
| `docs` | 文档 | `docs: update API reference` |
| `chore` | 构建/工具 | `chore: update russh to 0.49` |
| `perf` | 性能优化 | `perf(terminal): optimize glyph atlas cache` |
| `style` | 格式（不影响逻辑） | `style: run cargo fmt` |

**Scope 列表**：

| Scope | 对应模块 |
|-------|----------|
| `terminal` | 终端模拟器 |
| `ssh` | SSH 协议 |
| `telnet` | Telnet 协议 |
| `serial` | 串口协议 |
| `rdp` | RDP 协议 |
| `sftp` | SFTP 文件传输 |
| `transfer` | 文件传输引擎 |
| `session` | 会话管理 |
| `ui` | UI 框架 |
| `security` | 安全模块 |
| `script` | 脚本引擎 |
| `plugin` | 插件系统 |
| `core` | 核心后端 |
| `infra` | 基础设施 |

**规则**：
- subject 不超过 72 字符，英文小写开头，不加句号
- 每个提交只做一件事（原子提交）
- Breaking Change 在 footer 标注 `BREAKING CHANGE:`

### 3.3 Tag 与版本

```
v0.1.0  ← Phase 1 完成
v0.2.0  ← Phase 2 完成
v0.3.0  ← Phase 3 完成
v0.4.0  ← Phase 4 完成
v1.0.0  ← Phase 5 完成（正式版）
v1.1.0  ← Phase 6 完成（插件系统）
```

遵循 Semantic Versioning：`MAJOR.MINOR.PATCH`

---

## 4. 代码审查清单

### 4.1 通用检查项

```
□ 代码是否通过 cargo fmt 和 cargo clippy？
□ 是否有充分的单元测试？
□ 错误处理是否完整（无 unwrap/expect）？
□ 日志级别是否合理？
□ 是否有不必要的 clone() 或内存分配？
□ 异步代码是否有潜在的死锁或竞态？
□ 公开 API 是否有文档注释（///）？
```

### 4.2 前后端分离检查项

```
□ 后端代码是否引用了 GPUI 类型？（禁止）
□ 前端代码是否直接引用了 rshell-core？（禁止）
□ 跨层通信是否通过 Command/Event？
□ ViewModel 是否只包含 UI 状态 + 后端投影？
□ 跨边界数据结构是否实现了 Serialize/Deserialize？
```

### 4.3 安全相关检查项

```
□ 用户输入是否经过验证/清洗？
□ 敏感数据（密码、密钥）是否避免出现在日志中？
□ 加密操作是否使用经过验证的库（ring/rustls）？
□ 文件路径是否防止目录遍历攻击？
□ 网络输入是否有大小限制？
```

### 4.4 性能相关检查项

```
□ 是否有不必要的阻塞操作在 async 上下文中？
□ 大数据传输是否使用流式处理（非一次性加载）？
□ 循环中是否有可优化的 I/O 操作？
□ 是否使用了合适的集合类型（HashMap vs BTreeMap）？
```

---

## 5. 文档编写规范

### 5.1 代码注释

```rust
/// 终端服务 - 管理终端实例的生命周期
///
/// 负责创建、销毁终端，以及管理终端缓冲区。
/// 通过 Event 通知前端状态变化。
pub struct TerminalService {
    /// 活跃的终端实例映射
    terminals: HashMap<Uuid, TerminalInstance>,
}

impl TerminalService {
    /// 创建新的终端实例
    ///
    /// # Arguments
    /// * `config` - 终端配置，包含行列数和 TERM 环境变量
    ///
    /// # Returns
    /// 返回新终端的唯一标识符
    ///
    /// # Errors
    /// 当 PTY 创建失败时返回 `TerminalError::PtyError`
    ///
    /// # Examples
    /// ```
    /// let id = service.create_terminal(config)?;
    /// ```
    pub fn create_terminal(&self, config: TerminalConfig) -> Result<Uuid> {
        // ...
    }
}

// 规则：
// 1. 公开 API 必须有 /// 文档注释
// 2. 复杂逻辑用 // 行内注释
// 3. TODO/FIXME 标注格式：// TODO(作者): 描述
// 4. 不写显而易见的注释
```

### 5.2 设计文档模板

```markdown
# [功能名称] 设计文档

> **版本**：v1.0
> **日期**：YYYY-MM-DD
> **状态**：草稿 / 评审中 / 已批准

## 背景与目标
[为什么要做这个功能？解决什么问题？]

## 方案设计
[技术实现方案，含架构图、流程图]

## 接口定义
[新增/修改的 API]

## 数据模型
[新增/修改的数据结构]

## 风险与替代方案
[已知风险，备选方案]

## 测试计划
[如何验证方案正确性]
```

### 5.3 CHANGELOG 格式

```markdown
## [0.1.0] - 2026-XX-XX

### Added
- SSH2 连接（密码 + 公钥认证）
- 终端仿真（xterm-256color）
- 标签式多会话管理

### Fixed
- (无)

### Changed
- (无)
```

---

## 附录

### A. 常用 Cargo 命令

| 命令 | 用途 |
|------|------|
| `cargo fmt --all` | 格式化所有 crate |
| `cargo clippy --all -- -D warnings` | 严格静态分析 |
| `cargo test --workspace` | 运行所有测试 |
| `cargo test --package rshell-core` | 运行指定 crate 测试 |
| `cargo bench --workspace` | 运行基准测试 |
| `cargo build --release` | Release 构建 |
| `cargo deny check` | 许可证/安全审计 |

### B. IDE 配置建议

```json
// .vscode/settings.json
{
    "rust-analyzer.check.command": "clippy",
    "rust-analyzer.cargo.features": "all",
    "editor.formatOnSave": true,
    "[rust]": {
        "editor.defaultFormatter": "rust-lang.rust-analyzer"
    }
}
```
