# 现有功能缺陷修复实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复审查发现的 4 个现有功能缺陷：SFTP 传输进度不更新、主机密钥 `check_host_key` 死代码、可见操作静默失败、pause/resume 不可达状态未记录。

**Architecture:** 后端沿用「协议层只做 I/O、核心层做状态与事件」的既有分层——协议层新增可测的分块拷贝函数带进度回调，核心层用节流后的 mpsc forwarder 把回调落进任务表并广播 `TransferProgress`；速度随 `TransferTaskInfo` 快照下发，前端只消费快照。前端错误处理统一走「store 记录 `error` + 现有模板渲染」的既有模式，不新增全局错误总线。

**Tech Stack:** Rust 1.x / tokio 1.53 / russh-sftp、Tauri 2、Vue 3 + TypeScript + Pinia、Element Plus、vitest + @vue/test-utils、cargo clippy。

## Global Constraints

- 前端只通过 `src/ipc/client.ts` 与终端 Channel 调用后端；命令在 `src-tauri/src/commands.rs` 注册、在 `lib.rs` handler 列表公开。本计划不新增 Tauri 命令，因此不触碰这两处。
- 业务逻辑归 `rshell-core`，协议 I/O 归 `rshell-protocol`，`rshell-api` 只放共享类型；Rust Serde 外部标签枚举必须与 TypeScript 一致。
- 禁止 `as any`、`@ts-ignore`、`@ts-expect-error`；禁止空 `catch` 块；禁止删除失败测试来变绿。
- 不重新引入 RDP 或 Remote Forward（文档中出现 `RDP` 必须同句带「删除/移出/不提供/拒绝」）。
- 不记录私钥、口令、脚本凭据；本计划不改动任何凭据路径。
- 每条命令的执行位置与工具链：仓库根目录跑 `npm ...`；`src-tauri/` 内跑 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo ...`（本机 stable 自动更新冲突的既定绕法）。
- 完整验证清单（收尾任务执行）：`npm run typecheck`、`npm test`、`npm run test:scripts`、`npm run build`、`npm run check:docs`、`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。
- **不自动提交。** 每个任务给出「建议提交信息」，执行到该步时停下来等用户确认后再 `git commit`。
- 文档措辞以 `docs/08-incomplete-features.md`、`docs/09-macos-validation.md` 的体例为准：未真实环境验证的条目保持未勾选，不靠改写文档消缺口。

## 文件结构

| 职责 | 文件 |
| --- | --- |
| 分块拷贝 + 进度回调（协议层） | `src-tauri/crates/rshell-protocol/src/ssh/sftp.rs` |
| 进度节流、速度计算、任务表写入（核心层） | `src-tauri/crates/rshell-core/src/transfer/service.rs` |
| 任务快照类型（Rust） | `src-tauri/crates/rshell-api/src/types.rs` |
| 任务快照类型（TS） | `src/ipc/types.ts` |
| 传输任务 → 面板行 映射（新建，可单测） | `src/utils/transferItem.ts` |
| 队列刷新错误状态 | `src/App.vue` |
| 队列加载错误展示 | `src/components/TransferPanel.vue` |
| 主机密钥决策 store（nil 防御 + 错误保留） | `src/stores/hostKey.ts` |
| 主机密钥对话框错误展示 | `src/components/HostKeyMismatchDialog.vue` |
| 会话 store 错误落盘 | `src/stores/sessions.ts` |
| 删除不可达的 `check_host_key` | `src-tauri/crates/rshell-core/src/security/host_key_manager.rs` |
| pause/resume 无界面入口的说明 | `docs/08-incomplete-features.md` |
| 验证记录计数与结论 | `docs/09-macos-validation.md` |

新建测试：`tests/unit/transferItem.spec.ts`、`tests/unit/hostKeyStore.spec.ts`、`tests/unit/HostKeyMismatchDialog.spec.ts`、`tests/unit/sessionsStore.spec.ts`、`tests/unit/TransferPanel.spec.ts`；Rust 测试内联在 `sftp.rs` 与 `transfer/service.rs` 的 `#[cfg(test)] mod tests` 中。

---

# 缺陷 #1：SFTP 传输进度不更新

## 现状（已核实）

- `src-tauri/crates/rshell-protocol/src/ssh/sftp.rs:93,118` — `upload` 用 `tokio::fs::read` 一次性读完整文件再 `write_all`，`download` 用 `read_to_end` 再 `fs::write`，全程无进度回调，只在结束后返回总字节。
- `src-tauri/crates/rshell-core/src/transfer/service.rs:437` — `update_progress`（唯一含速度计算的函数）**全仓库零调用点**，也无测试。
- `service.rs:305-349` — 传输结束后才写 `bytes_transferred` 并发 `TransferProgress { total: bytes, speed_bps: 0.0 }`。
- `src/App.vue:86` — `speed: 0` 硬编码，事件里的 `speed_bps` 无人消费。
- 结果：进度条 0% 直到完成瞬间跳 100%，速度与剩余时间恒为 `—`。

## 方向（用户已确认）

完整修复：协议层加进度回调 → 核心层节流 + 速度计算 → `TransferTaskInfo` 携带 `speed_bps` → 前端消费。

---

### Task 1: 协议层分块拷贝函数 `copy_with_progress`

**Files:**
- Modify: `src-tauri/crates/rshell-protocol/src/ssh/sftp.rs`（在 `impl SftpClient` 之后、文件末尾追加自由函数与 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `crate::ProtocolError`（`ProtocolError::ProtocolError(String)` 变体，`thiserror` 派生 `Debug`）；`tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt}`（文件顶部已 `use` 后两者）。
- Produces: `async fn copy_with_progress<R, W, F>(reader: &mut R, writer: &mut W, total: u64, chunk_size: usize, progress: &mut F) -> Result<u64, ProtocolError>`，其中 `R: AsyncRead + Unpin`、`W: AsyncWrite + Unpin`、`F: FnMut(u64, u64)`。Task 3 的 `upload`/`download` 依赖此签名。

- [ ] **Step 1: 写失败测试**

在 `sftp.rs` 文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copy_reports_monotonic_progress_and_exact_final_total() {
        let payload = vec![7u8; 1000];
        let mut reader: &[u8] = payload.as_slice();
        let mut writer: Vec<u8> = Vec::new();
        let mut calls: Vec<(u64, u64)> = Vec::new();

        let copied = copy_with_progress(&mut reader, &mut writer, 1000, 128, &mut |done, total| {
            calls.push((done, total));
        })
        .await
        .unwrap();

        assert_eq!(copied, 1000);
        assert_eq!(writer.len(), 1000);
        assert!(calls.len() > 1, "1000 字节 / 128 分块应产生多次回调");
        assert_eq!(calls[0], (0, 1000), "开始前应先报一次 (0, total)");
        let mut prev = 0u64;
        for (done, total) in &calls {
            assert_eq!(*total, 1000);
            assert!(*done >= prev, "进度不允许回退");
            prev = *done;
        }
        assert_eq!(calls.last().copied(), Some((1000, 1000)));
    }

    #[tokio::test]
    async fn copy_accepts_unknown_total_but_still_reports_bytes() {
        let payload = vec![1u8; 300];
        let mut reader: &[u8] = payload.as_slice();
        let mut writer: Vec<u8> = Vec::new();
        let mut last = (0u64, 0u64);

        let copied = copy_with_progress(&mut reader, &mut writer, 0, 64, &mut |d, t| last = (d, t))
            .await
            .unwrap();

        assert_eq!(copied, 300);
        assert_eq!(last.0, 300);
    }

    #[tokio::test]
    async fn copy_rejects_when_source_shrinks_mid_transfer() {
        let payload = vec![0u8; 500];
        let mut reader: &[u8] = payload.as_slice();
        let mut writer: Vec<u8> = Vec::new();

        let err = copy_with_progress(&mut reader, &mut writer, 1000, 64, &mut |_, _| {})
            .await
            .unwrap_err();

        assert!(
            format!("{err:?}").contains("source changed"),
            "实际错误: {err:?}"
        );
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test -p rshell-protocol copy_`
Expected: FAIL，编译错误 `cannot find function copy_with_progress in this scope`

- [ ] **Step 3: 实现 `copy_with_progress`**

在 `sftp.rs` 的 `impl SftpClient { ... }` 之后追加：

```rust
/// 分块拷贝，进度变化时回调 `(bytes_done, total)`。
///
/// 开始前先回调一次 `(0, total)`，返回前保证发出终帧；
/// 声明的 `total` 与实际拷贝字节数不一致（源在传输期间被改写）时报错，
/// 避免远端留下被截断的文件。
async fn copy_with_progress<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    total: u64,
    chunk_size: usize,
    progress: &mut F,
) -> Result<u64, ProtocolError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
    F: FnMut(u64, u64),
{
    progress(0, total);

    let mut buf = vec![0u8; chunk_size.max(1)];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buf).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("read failed during copy: {e}"))
        })?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("write failed during copy: {e}"))
        })?;
        done += n as u64;
        progress(done, total);
    }
    writer.flush().await.map_err(|e| {
        ProtocolError::ProtocolError(format!("flush failed during copy: {e}"))
    })?;

    if total > 0 && done != total {
        return Err(ProtocolError::ProtocolError(format!(
            "source changed during transfer: expected {total} bytes, copied {done}"
        )));
    }
    Ok(done)
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test -p rshell-protocol copy_`
Expected: PASS，3 项

（依据：tokio 1.53.1 提供 `impl AsyncRead for &[u8]` 与 `impl AsyncWrite for Vec<u8>`，`rshell-protocol` 已通过 `#[tokio::test]` 使用 tokio 宏。）

- [ ] **Step 5: 提交（等用户确认）**

建议提交信息：`feat(sftp): add chunked copy helper with progress callbacks`

---

### Task 2: 核心层节流器、进度落表重构与 `speed_bps` 贯通

**Files:**
- Modify: `src-tauri/crates/rshell-core/src/transfer/service.rs`（`TransferTask` 结构、`From` 实现、两处 `enqueue_*` 的字面量、`update_progress` 重构、新增 `ProgressThrottle`、`mod tests`）
- Modify: `src-tauri/crates/rshell-api/src/types.rs:98-108`（`TransferTaskInfo`）
- Modify: `src/ipc/types.ts:93-103`（TS 侧同名接口）

**Interfaces:**
- Consumes: 现有 `TransferService { tasks: Arc<RwLock<HashMap<Uuid, TransferTask>>>, event_bus: Arc<EventBus> }`、`EventBus::subscribe(Fn(&AppEvent))`。
- Produces（Task 3、4 依赖）:
  - `const PROGRESS_MIN_INTERVAL: std::time::Duration`（200ms）
  - `struct ProgressThrottle { ... }` + `ProgressThrottle::new(Duration)` + `fn should_emit(&mut self, done: u64, total: u64) -> bool`
  - `async fn TransferService::apply_progress(tasks: &Arc<RwLock<HashMap<Uuid, TransferTask>>>, event_bus: &Arc<EventBus>, task_id: Uuid, bytes_transferred: u64, total_bytes: u64)`
  - `pub async fn update_progress(&self, task_id: Uuid, bytes: u64, total: u64) -> Result<(), CoreError>`（委托给 `apply_progress`）
  - `TransferTask.speed_bps: f64`、`TransferTaskInfo.speed_bps: f64`、TS `TransferTaskInfo.speed_bps: number`

- [ ] **Step 1: 写失败测试**

在 `service.rs` 的 `mod tests` 中追加：

```rust
    #[test]
    fn progress_throttle_emits_first_immediately_and_final_frame_always() {
        let mut t = ProgressThrottle::new(std::time::Duration::from_secs(60));
        assert!(t.should_emit(0, 1000), "首帧必须立即发出");
        assert!(!t.should_emit(128, 1000), "间隔内的中间帧要被节流");
        assert!(!t.should_emit(256, 1000));
        assert!(t.should_emit(1000, 1000), "终帧必须无条件发出");

        let mut fast = ProgressThrottle::new(std::time::Duration::ZERO);
        assert!(fast.should_emit(1, 1000));
        assert!(fast.should_emit(2, 1000));
    }

    #[tokio::test]
    async fn apply_progress_updates_task_and_publishes_nonzero_speed() {
        let event_bus = Arc::new(EventBus::new());
        let svc = TransferService::new(event_bus.clone());
        let id = Uuid::new_v4();
        {
            let mut tasks = svc.tasks.write().await;
            let mut task = make_task(id, TransferTaskState::Transferring);
            // 500ms 前进过 0 字节 → 500 字节 / 0.5s = 1000 B/s
            task.last_update = Some(std::time::Instant::now() - std::time::Duration::from_millis(500));
            task.last_bytes = 0;
            tasks.insert(id, task);
        }

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        event_bus.subscribe(move |event| {
            if let AppEvent::TransferProgress { bytes, total, speed_bps, .. } = event {
                sink.lock().unwrap().push((*bytes, *total, *speed_bps));
            }
        });

        TransferService::apply_progress(&svc.tasks, &svc.event_bus, id, 500, 1000).await;

        {
            let tasks = svc.tasks.read().await;
            let task = tasks.get(&id).unwrap();
            assert_eq!(task.bytes_transferred, 500);
            assert_eq!(task.total_bytes, 1000);
            assert!(task.speed_bps > 0.0, "speed_bps 应为正数");
        }

        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].0, events[0].1), (500, 1000));
        assert!(events[0].2 > 0.0, "广播出去的速度也要是正数");
    }
```

（`make_task(id, state)` 是 `service.rs:564` 已有的测试构造器，本任务 Step 3 会给它补 `speed_bps: 0.0`。）

- [ ] **Step 2: 跑测试确认失败**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test -p rshell-core transfer::service`
Expected: FAIL，编译错误 `cannot find struct/function ProgressThrottle / apply_progress`

- [ ] **Step 3: 加字段、重构 `update_progress`、实现节流器**

3a. `TransferTask` 结构（`service.rs:61-77`）末尾加字段：

```rust
    /// 当前传输速度（字节/秒）。仅在 Transferring 期间有值，其余状态由前端归零显示。
    pub speed_bps: f64,
```

3b. 四处 `TransferTask { ... }` 字面量补 `speed_bps: 0.0,`：`service.rs:165`（enqueue_upload）、`service.rs:214`（enqueue_download）、`service.rs:565`（测试 `make_task`）、`service.rs:733`、`service.rs:750`（测试构造）。

3c. `From<TransferTask> for TransferTaskInfo`（`service.rs:90-114`）补一行：

```rust
            speed_bps: task.speed_bps,
```

3d. `rshell-api/src/types.rs` 的 `TransferTaskInfo` 补字段：

```rust
    /// 当前传输速度（字节/秒），仅 Transferring 状态有值
    pub speed_bps: f64,
```

3e. `src/ipc/types.ts` 的 `TransferTaskInfo` 补字段（与 Serde snake_case 一致）：

```ts
  speed_bps: number;
```

3f. 把 `update_progress`（`service.rs:436-473`）重构为委托 + 新增 `apply_progress` 与节流器：

```rust
/// 进度事件最短发布间隔：避免每个分块都触发一次前端全量队列刷新。
const PROGRESS_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// 进度事件节流：间隔内的帧丢弃，首帧与终帧无条件发出。
struct ProgressThrottle {
    last_sent: Option<std::time::Instant>,
    min_interval: std::time::Duration,
}

impl ProgressThrottle {
    fn new(min_interval: std::time::Duration) -> Self {
        Self { last_sent: None, min_interval }
    }

    fn should_emit(&mut self, done: u64, total: u64) -> bool {
        let final_frame = total > 0 && done >= total;
        let now = std::time::Instant::now();
        let emit = final_frame
            || match self.last_sent {
                None => true,
                Some(sent) => sent.elapsed() >= self.min_interval,
            };
        if emit {
            self.last_sent = Some(now);
        }
        emit
    }
}
```

```rust
    /// 更新传输进度（写任务表 + 广播 `TransferProgress`）
    ///
    /// 抽成关联函数是为了让 execute_transfer 里的 forwarder 任务
    /// 只持有 `tasks` / `event_bus` 的克隆即可复用同一逻辑。
    async fn apply_progress(
        tasks: &Arc<RwLock<HashMap<Uuid, TransferTask>>>,
        event_bus: &Arc<EventBus>,
        task_id: Uuid,
        bytes_transferred: u64,
        total_bytes: u64,
    ) {
        let mut tasks = tasks.write().await;
        let Some(task) = tasks.get_mut(&task_id) else {
            return;
        };

        // 速度 = 自上次更新以来的字节增量 / 间隔
        let speed_bps = match (task.last_update, task.last_bytes) {
            (Some(last_update), last_bytes) => {
                let elapsed = last_update.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    bytes_transferred.saturating_sub(last_bytes) as f64 / elapsed
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };

        task.bytes_transferred = bytes_transferred;
        task.total_bytes = total_bytes;
        task.last_update = Some(std::time::Instant::now());
        task.last_bytes = bytes_transferred;
        task.speed_bps = speed_bps;

        event_bus.publish(AppEvent::TransferProgress {
            task_id,
            bytes: bytes_transferred,
            total: total_bytes,
            speed_bps,
        });
    }

    /// 更新传输进度
    pub async fn update_progress(
        &self,
        task_id: Uuid,
        bytes_transferred: u64,
        total_bytes: u64,
    ) -> Result<(), CoreError> {
        Self::apply_progress(&self.tasks, &self.event_bus, task_id, bytes_transferred, total_bytes)
            .await;
        Ok(())
    }
```

> 注意：原实现是在更新 `last_update`/`last_bytes` **之前**用旧值算速度，重构后保持等价语义（先用旧 `last_update` 算增量，再覆盖）。

- [ ] **Step 4: 跑测试确认通过**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test -p rshell-core transfer::service`
Expected: PASS（新增 2 项 + 既有项全绿）

- [ ] **Step 5: 编译与 lint 兜底**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 警告（若 `TransferTaskInfo` 新字段在别处被穷举构造而漏改，此处会报错，逐个补 `speed_bps: 0.0` 或真值）

- [ ] **Step 6: 提交（等用户确认）**

建议提交信息：`feat(transfer): add throttled progress plumbing and speed_bps snapshot field`

---

### Task 3: upload/download 接进度回调 + forwarder 接线

**Files:**
- Modify: `src-tauri/crates/rshell-protocol/src/ssh/sftp.rs:92-145`（`upload`、`download`）
- Modify: `src-tauri/crates/rshell-core/src/transfer/service.rs:290-354`（`execute_transfer` 的 `tokio::spawn` 块）

**Interfaces:**
- Consumes: Task 1 的 `copy_with_progress`；Task 2 的 `ProgressThrottle`、`PROGRESS_MIN_INTERVAL`、`TransferService::apply_progress`。
- Produces: `pub async fn upload<F>(&self, local: &PathBuf, remote: &str, progress: F) -> Result<u64, ProtocolError>` 与 `pub async fn download<F>(&self, remote: &str, local: &PathBuf, progress: F) -> Result<u64, ProtocolError>`，其中 `F: FnMut(u64, u64)`（借用方式传 `&mut progress`）。

- [ ] **Step 1: 改写 `upload`**

```rust
    /// 上传本地文件到远程（分块写入，边传边回调进度）
    pub async fn upload<F>(&self, local: &PathBuf, remote: &str, mut progress: F) -> Result<u64, ProtocolError>
    where
        F: FnMut(u64, u64),
    {
        info!(local = %local.display(), remote = %remote, "Uploading file");

        let total = tokio::fs::metadata(local).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to stat local file: {}", e))
        })?
        .len();

        let mut source = tokio::fs::File::open(local).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to open local file: {}", e))
        })?;

        let mut target = self.session.create(remote).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to create remote file: {}", e))
        })?;

        let copied = copy_with_progress(&mut source, &mut target, total, CHUNK_SIZE, &mut progress).await?;
        drop(target);

        info!(remote = %remote, bytes = copied, "Upload completed");
        Ok(copied)
    }
```

- [ ] **Step 2: 改写 `download`**

```rust
    /// 下载远程文件到本地（分块写入，边传边回调进度）
    pub async fn download<F>(&self, remote: &str, local: &PathBuf, mut progress: F) -> Result<u64, ProtocolError>
    where
        F: FnMut(u64, u64),
    {
        info!(remote = %remote, local = %local.display(), "Downloading file");

        let total = self.session.metadata(remote).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to stat remote file: {}", e))
        })?
        .len();

        let mut source = self.session.open(remote).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to open remote file: {}", e))
        })?;

        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                ProtocolError::ProtocolError(format!("Failed to create local dir: {}", e))
            })?;
        }
        let mut target = tokio::fs::File::create(local).await.map_err(|e| {
            ProtocolError::ProtocolError(format!("Failed to create local file: {}", e))
        })?;

        let copied = copy_with_progress(&mut source, &mut target, total, CHUNK_SIZE, &mut progress).await?;

        info!(remote = %remote, bytes = copied, "Download completed");
        Ok(copied)
    }
```

同时在 `sftp.rs` 顶部（`use tracing::{debug, info};` 之后）加：

```rust
/// 单次读写分块大小（64 KiB）
const CHUNK_SIZE: usize = 64 * 1024;
```

> 变更点说明：原实现先整读进内存再一次性写；新实现流式分块，内存占用从「整个文件」降为 64 KiB，且远端文件在传输期间被外部改写时由 `copy_with_progress` 报错而非静默截断。原 `download` 中「确保本地目录存在」的行为保留。

- [ ] **Step 3: 改写 `execute_transfer` 接线**

替换 `service.rs` 中 `tokio::spawn(async move { ... })` 的开头到 `match result` 之前的部分：

```rust
        tokio::spawn(async move {
            // SFTP 拷贝循环里只能同步回调（不能在回调里 await 写锁），
            // 所以回调只做节流后投递，由独立 forwarder 任务落表并广播。
            let (progress_tx, mut progress_rx) =
                tokio::sync::mpsc::unbounded_channel::<(u64, u64)>();
            let forward_tasks = tasks.clone();
            let forward_bus = event_bus.clone();
            let forwarder = tokio::spawn(async move {
                while let Some((bytes, total)) = progress_rx.recv().await {
                    TransferService::apply_progress(
                        &forward_tasks,
                        &forward_bus,
                        task_id,
                        bytes,
                        total,
                    )
                    .await;
                }
            });

            let mut throttle = ProgressThrottle::new(PROGRESS_MIN_INTERVAL);
            let mut progress = move |done: u64, total: u64| {
                if throttle.should_emit(done, total) {
                    let _ = progress_tx.send((done, total));
                }
            };

            let result = async {
                let ssh = ssh_client.read().await;
                let channel = ssh
                    .open_sftp_channel()
                    .await
                    .map_err(|e| format!("Failed to open SFTP channel: {}", e))?;

                let sftp = SftpClient::new(channel)
                    .await
                    .map_err(|e| format!("Failed to create SFTP client: {}", e))?;

                match task.direction {
                    TransferDirection::Upload => {
                        sftp.upload(&task.local_path, &task.remote_path, &mut progress)
                            .await
                            .map_err(|e| format!("Upload failed: {}", e))?;
                    }
                    TransferDirection::Download => {
                        sftp.download(&task.remote_path, &task.local_path, &mut progress)
                            .await
                            .map_err(|e| format!("Download failed: {}", e))?;
                    }
                }

                Ok::<(), String>(())
            }
            .await;

            // 关闭发送端 → forwarder 排空最后一批进度后自行退出
            drop(progress);
            let _ = forwarder.await;

            match result {
                Ok(()) => {
                    let mut tasks = tasks.write().await;
                    if let Some(t) = tasks.get_mut(&task_id) {
                        t.state = TransferTaskState::Completed;
                    }
                    event_bus.publish(AppEvent::TransferCompleted { task_id });
                    event_bus.publish(AppEvent::TransferQueueChanged);
                    info!(task_id = %task_id, "Transfer completed");
                }
                Err(e) => { /* 沿用现有失败分支，见 Step 4 */ }
            }
        });
```

**Step 4 必读**：原代码在 `Ok(())` 分支之前有两段「传完才写 `bytes_transferred` 并发 `TransferProgress { speed_bps: 0.0 }`」的收尾（`service.rs:312-326` 与 `334-348`），现在**必须删除**——终帧已由 `copy_with_progress` → throttle → forwarder 写入，重复写会用 `total: bytes` 覆盖真实文件大小。失败分支（`mark_failed` / `TransferFailed` / `TransferQueueChanged`）原样保留。

- [ ] **Step 4: 编译 + 全量测试 + lint**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo clippy --workspace --all-targets -- -D warnings && RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test --workspace`
Expected: clippy 0 警告；全部测试通过

- [ ] **Step 5: 手工验证说明（写进验证记录，不改代码）**

真实 SSH/SFTP 服务器未提供，本轮不勾选 `docs/09` 中的 SFTP 真实验收项；本任务只保证「分块进度链路的单元测试 + 编译/lint」通过。

- [ ] **Step 6: 提交（等用户确认）**

建议提交信息：`feat(transfer): stream sftp transfers with throttled progress events`

---

### Task 4: 前端消费 `speed_bps`

**Files:**
- Create: `src/utils/transferItem.ts`
- Create: `tests/unit/transferItem.spec.ts`
- Modify: `src/App.vue`（`toTransferItem` 定义与 `refreshTransfers` 调用处）

**Interfaces:**
- Consumes: `src/ipc/types.ts` 的 `TransferTaskInfo`（含 Task 2 新增的 `speed_bps: number`）；`src/components/TransferPanel.vue` 导出的 `TransferItem`、`TransferPhase` 类型。
- Produces: `export function toTransferItem(task: TransferTaskInfo): TransferItem`

- [ ] **Step 1: 写失败测试**

```ts
import { describe, expect, it } from "vitest";
import { toTransferItem } from "../../src/utils/transferItem";
import type { TransferTaskInfo } from "../../src/ipc/types";

function task(overrides: Partial<TransferTaskInfo> = {}): TransferTaskInfo {
  return {
    id: "task-1",
    session_id: "session-1",
    direction: "Upload",
    local_path: "/Users/test/a.bin",
    remote_path: "/data/a.bin",
    state: "Transferring",
    bytes_transferred: 0,
    total_bytes: 0,
    speed_bps: 0,
    error_message: null,
    ...overrides,
  };
}

describe("toTransferItem", () => {
  it("maps live backend speed and progress for a transferring task", () => {
    const row = toTransferItem(task({ bytes_transferred: 500, total_bytes: 1000, speed_bps: 1000 }));
    expect(row.phase).toBe("active");
    expect(row.progress).toBeCloseTo(0.5);
    expect(row.speed).toBe(1000);
  });

  it("zeroes speed for terminal states so the panel shows an em dash", () => {
    expect(toTransferItem(task({ state: "Completed", bytes_transferred: 10, total_bytes: 10, speed_bps: 999 })).speed).toBe(0);
    expect(toTransferItem(task({ state: "Failed", error_message: "boom" })).phase).toBe("failed");
  });

  it("never divides by zero when total is still unknown", () => {
    expect(toTransferItem(task({ bytes_transferred: 0, total_bytes: 0 })).progress).toBe(0);
  });

  it("uses the remote file name as display name", () => {
    expect(toTransferItem(task()).name).toBe("a.bin");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -- transferItem`
Expected: FAIL，`Cannot find module .../src/utils/transferItem`

- [ ] **Step 3: 实现 util 并接回 App.vue**

`src/utils/transferItem.ts`：

```ts
/**
 * 后端传输任务快照 → 面板行。
 *
 * 速度只在 Transferring 状态展示：终态下残留的最后一次采样速度没有意义，
 * 归零后 TransferPanel 的 fmtSpeed(0) 会显示 "—"。
 */
import type { TransferTaskInfo } from "../ipc/types";
import type { TransferItem, TransferPhase } from "../components/TransferPanel.vue";

const PHASE_BY_STATE: Record<TransferTaskInfo["state"], TransferPhase> = {
  Pending: "queued",
  Transferring: "active",
  Paused: "paused",
  Completed: "done",
  Failed: "failed",
  Cancelled: "cancelled",
};

export function toTransferItem(task: TransferTaskInfo): TransferItem {
  return {
    id: task.id,
    name: task.remote_path.split("/").pop() || task.remote_path,
    phase: PHASE_BY_STATE[task.state],
    progress: task.total_bytes ? task.bytes_transferred / task.total_bytes : 0,
    size: task.total_bytes,
    local: task.local_path,
    remote: task.remote_path,
    speed: task.state === "Transferring" ? task.speed_bps : 0,
    error: task.error_message,
  };
}
```

`src/App.vue`：删除本地 `function toTransferItem(...)`（约 71-88 行），改为 `import { toTransferItem } from "./utils/transferItem";`（放在其它 `./utils/` import 旁）。

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test -- transferItem`
Expected: PASS，4 项

- [ ] **Step 5: 类型检查**

Run: `npm run typecheck`
Expected: 通过

- [ ] **Step 6: 提交（等用户确认）**

建议提交信息：`feat(frontend): surface transfer speed from backend snapshot`

---

# 缺陷 #2：`check_host_key` 死代码 + 会弹出必然失败的决策框

## 现状（已核实）

- `host_key_manager.rs:158` 定义 `check_host_key`，全仓库唯一调用点是它自己的测试 `permanent_trust_persists_across_manager_restart`（`host_key_manager.rs:319`）。
- 它在 `host_key_manager.rs:177` 发布 `HostKeyMismatch { decision_id: Uuid::nil(), ...}`，注释要求 UI「作为严重告警展示，不应回 `DecideHostKey`」。
- `src/stores/hostKey.ts:33` 不区分 nil：任何 `HostKeyMismatch` 都会变成可决策对话框；点任一按钮 → `command_dispatcher.rs:462` 返回 `NotFound`，而 store 在 `await` **之前**就清空 `current`，用户看到对话框凭空消失 + 无提示。
- 真实握手链路是 `rshell-protocol/src/ssh/client.rs:256 check_server_key` → `verify_known_hosts`（密钥变化时回传 `expected`，带真实 `decision_id`，60 秒超时保护），**不受本任务影响**。

---

### Task 5: 删除 `check_host_key` 并改写持久化测试

**Files:**
- Modify: `src-tauri/crates/rshell-core/src/security/host_key_manager.rs`（删除 `check_host_key` 方法体、`use rshell_api::events::AppEvent;` 视情况、改写测试）

**Interfaces:**
- Consumes: `HostKeyManager` 的私有字段 `entries: RwLock<HashMap<String, HostKeyEntry>>`（同文件测试可访问）。
- Produces: 无对外接口变化（该方法本就不可达）。

- [ ] **Step 1: 写失败测试（先改测试，锁定期望）**

把 `mod tests` 中的 `permanent_trust_persists_across_manager_restart` 改为通过 `entries` 直接断言磁盘恢复，不再调用 `check_host_key`：

```rust
    #[tokio::test]
    async fn permanent_trust_persists_across_manager_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let bus = Arc::new(EventBus::new());
        let manager = HostKeyManager::new(path.clone(), bus.clone());
        manager
            .trust_host_key("example.test", 2222, "ssh-ed25519", "AAAAkey")
            .await
            .unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("[example.test]:2222 ssh-ed25519 AAAAkey"));

        // 重启后应从磁盘恢复条目（握手期的比对由协议层 verify_known_hosts 负责）
        let restored = HostKeyManager::new(path, bus);
        let entries = restored.entries.read().unwrap();
        let entry = entries.get("example.test:2222").expect("重启后条目应被加载");
        assert_eq!(entry.fingerprint, "AAAAkey");
        assert_eq!(entry.trust_level, TrustLevel::Trusted);
        drop(entries);
    }
```

- [ ] **Step 2: 跑测试确认失败/编译报错**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test -p rshell-core permanent_trust_persists`
Expected: 此时 `check_host_key` 仍存在，测试 PASS（说明新断言等价）

- [ ] **Step 3: 删除 `check_host_key` 方法**

删除 `host_key_manager.rs:150-204` 的整个 `pub async fn check_host_key(...)`（含其 doc 注释）。**保留** `check_host_key` 内部那段「已知条目但指纹不一致」的语义说明——它是历史决策记录，改以注释形式移到 `verify_known_hosts` 相关说明处会牵动协议层，因此本任务只在删除处留一行指引：

```rust
// 已知条目指纹不一致（可能中间人）由协议层 rshell-protocol::ssh::client
// 的 verify_known_hosts → check_server_key 决策链处理，不再在此重复判定。
```

- [ ] **Step 4: 清理编译器报出的未使用项**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 警告。若报 `unused import: rshell_api::events::AppEvent`（原仅 `check_host_key` 使用），删除 `host_key_manager.rs:16` 的该 `use`；若 `warn!`/`info!` 宏未使用，同步收敛 import。**不要**为了消警告而加 `#[allow(dead_code)]`。

- [ ] **Step 5: 跑测试确认通过**

Run: `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test -p rshell-core`
Expected: PASS（`permanent_trust_persists_across_manager_restart` 走新断言）

- [ ] **Step 6: 提交（等用户确认）**

建议提交信息：`refactor(host-key): remove unreachable check_host_key path`

---

### Task 6: hostKey store —— nil 决策防御 + 决策失败保留对话框

**Files:**
- Create: `tests/unit/hostKeyStore.spec.ts`
- Modify: `src/stores/hostKey.ts`

**Interfaces:**
- Consumes: `subscribeAppEvents`（`src/ipc/events.ts`，`async (handler) => unlisten`）、`decideHostKey`（`src/ipc/client.ts`）。
- Produces: store 新增导出 `error: Ref<string | null>`；`trustOnce()` / `trustPermanent()` / `reject()` 改为 async，失败时**不清空** `current` 且写入 `error`；`decision_id` 为 nil UUID 的事件被忽略（`current` 保持不变）。`HostKeyMismatchDialog`（Task 7）依赖 `store.error`。

- [ ] **Step 1: 写失败测试**

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const decideHostKey = vi.fn();
const handlers: Array<(event: unknown) => void> = [];

vi.mock("../../src/ipc/client", () => ({
  decideHostKey: (...args: unknown[]) => decideHostKey(...args),
}));
vi.mock("../../src/ipc/events", () => ({
  subscribeAppEvents: vi.fn(async (handler: (event: unknown) => void) => {
    handlers.push(handler);
    return () => {};
  }),
}));

const { useHostKeyStore } = await import("../../src/stores/hostKey");

const NIL_UUID = "00000000-0000-0000-0000-000000000000";

function mismatch(decision_id: string) {
  return {
    HostKeyMismatch: {
      decision_id,
      host: "example.test",
      port: 22,
      key_type: "ssh-ed25519",
      expected: "",
      received: "SHA256:aaa",
      public_key_blob: "AAAA",
    },
  };
}

describe("hostKey store", () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    handlers.length = 0;
    decideHostKey.mockReset();
    const store = useHostKeyStore();
    await store.subscribeEvents();
  });

  it("opens the decision dialog for a real decision id", async () => {
    handlers.at(-1)!(mismatch("11111111-1111-1111-1111-111111111111"));
    expect(useHostKeyStore().current?.decision_id).toBe("11111111-1111-1111-1111-111111111111");
  });

  it("ignores a nil decision id so no doomed dialog is shown", async () => {
    handlers.at(-1)!(mismatch(NIL_UUID));
    expect(useHostKeyStore().current).toBeNull();
  });

  it("keeps the dialog open and records the error when the decision fails", async () => {
    handlers.at(-1)!(mismatch("22222222-2222-2222-2222-222222222222"));
    const store = useHostKeyStore();
    expect(store.current).not.toBeNull();

    decideHostKey.mockRejectedValueOnce("decision rejected");
    await store.trustPermanent();

    expect(store.current).not.toBeNull();
    expect(store.error).toContain("decision rejected");
    expect(store.history).toHaveLength(0);
    expect(decideHostKey).toHaveBeenCalledWith("22222222-2222-2222-2222-222222222222", true, true);
  });

  it("clears the dialog only after a successful decision", async () => {
    handlers.at(-1)!(mismatch("33333333-3333-3333-3333-333333333333"));
    const store = useHostKeyStore();
    decideHostKey.mockResolvedValueOnce(undefined);

    await store.reject();

    expect(store.current).toBeNull();
    expect(store.error).toBeNull();
    expect(store.history).toHaveLength(1);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -- hostKeyStore`
Expected: FAIL —— `store.error` 不存在、nil 事件被当成普通决策、失败时 `current` 已被清空

- [ ] **Step 3: 实现**

`src/stores/hostKey.ts` 修改：

```ts
import { defineStore } from "pinia";
import { ref } from "vue";
import { subscribeAppEvents } from "../ipc/events";
import { decideHostKey } from "../ipc/client";

export interface HostKeyRequest {
  decision_id: string;
  host: string;
  port: number;
  key_type: string;
  expected: string;
  received: string;
  public_key_blob: string;
}

const NIL_DECISION_ID = "00000000-0000-0000-0000-000000000000";

export const useHostKeyStore = defineStore("hostKey", () => {
  const current = ref<HostKeyRequest | null>(null);
  const history = ref<HostKeyRequest[]>([]);
  const error = ref<string | null>(null);
  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (typeof event === "string" || !("HostKeyMismatch" in event)) return;
      const request = event.HostKeyMismatch;
      // nil decision_id 表示「已知密钥指纹变化」的单向告警，没有可回写的
      // 决策通道，弹决策框只会让按钮全部失败 —— 这里显式忽略。
      if (request.decision_id === NIL_DECISION_ID) return;
      error.value = null;
      current.value = request;
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() { subscriptionGeneration++; unlisten?.(); unlisten = null; }

  /** 提交决策；成功才收起对话框，失败保留对话框并记录错误供界面展示。 */
  async function submit(decision_id: string, accept: boolean, permanent: boolean) {
    error.value = null;
    try {
      await decideHostKey(decision_id, accept, permanent);
    } catch (e) {
      error.value = String(e);
      return;
    }
    const req = current.value;
    if (req) history.value.push(req);
    current.value = null;
  }

  async function trustOnce() { if (current.value) await submit(current.value.decision_id, true, false); }
  async function trustPermanent() { if (current.value) await submit(current.value.decision_id, true, true); }
  async function reject() { if (current.value) await submit(current.value.decision_id, false, false); }

  function dismiss() { current.value = null; error.value = null; }

  return { current, history, error, subscribeEvents, disposeEvents, trustOnce, trustPermanent, reject, dismiss };
});
```

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test -- hostKeyStore`
Expected: PASS，4 项

- [ ] **Step 5: 提交（等用户确认）**

建议提交信息：`fix(host-key): ignore nil decisions and keep dialog open on failure`

---

### Task 7: HostKeyMismatchDialog 展示 store.error

**Files:**
- Create: `tests/unit/HostKeyMismatchDialog.spec.ts`
- Modify: `src/components/HostKeyMismatchDialog.vue`（`<p class="warning">` 之后插入错误提示 + 对应样式）

**Interfaces:**
- Consumes: Task 6 的 `store.error`。
- Produces: 无（纯展示）。

- [ ] **Step 1: 写失败测试**

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";

vi.mock("../../src/ipc/client", () => ({ decideHostKey: vi.fn() }));

const { useHostKeyStore } = await import("../../src/stores/hostKey");
import HostKeyMismatchDialog from "../../src/components/HostKeyMismatchDialog.vue";

const dialogStub = {
  template: '<div><slot /><footer><slot name="footer" /></footer></div>',
};

function mountDialog() {
  return mount(HostKeyMismatchDialog, {
    global: { stubs: { "el-dialog": dialogStub, "el-button": true } },
  });
}

describe("HostKeyMismatchDialog", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("renders nothing when there is no pending request", () => {
    expect(mountDialog().text()).toBe("");
  });

  it("surfaces the store error so a failed decision is visible", async () => {
    const store = useHostKeyStore();
    store.current = {
      decision_id: "44444444-4444-4444-4444-444444444444",
      host: "example.test",
      port: 22,
      key_type: "ssh-ed25519",
      expected: "",
      received: "SHA256:aaa",
      public_key_blob: "AAAA",
    };
    store.error = "Host key decision is no longer pending";

    const text = mountDialog().text();
    expect(text).toContain("example.test");
    expect(text).toContain("Host key decision is no longer pending");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -- HostKeyMismatchDialog`
Expected: FAIL —— 第二个用例找不到错误文案

- [ ] **Step 3: 实现**

在 `src/components/HostKeyMismatchDialog.vue` 的 `<p class="warning">⚠️ ...</p>` 之后、`</template>`（`v-if="store.current"` 那个）结束之前插入：

```html
      <p v-if="store.error" class="decision-error" role="alert">
        决策提交失败：{{ store.error }}。连接会在超时后被拒绝，请重试或检查后端状态。
      </p>
```

`<style scoped>` 中追加（与既有 `.warning` 同级）：

```css
.decision-error {
  margin: 12px 0 0;
  padding: 8px 10px;
  border: 1px solid var(--rs-danger, #f56c6c);
  border-radius: 4px;
  color: var(--rs-danger, #f56c6c);
  font-size: 12px;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test -- HostKeyMismatchDialog`
Expected: PASS，2 项

- [ ] **Step 5: 提交（等用户确认）**

建议提交信息：`feat(host-key): show decision failures inside the trust dialog`

---

# 缺陷 #3：可见操作静默失败

## 现状（已核实）

| 位置 | 现状 |
| --- | --- |
| `src/components/SessionList.vue:113` | 右键「断开」→ `store.disconnect(...).catch(console.warn)` |
| `src/components/SessionList.vue:117` | 右键「删除」→ `store.delete(...).catch(console.warn)` |
| `src/stores/sessions.ts:66-75` | `disconnect` / `deleteSessionById` 不写 `store.error`（`refresh` 才写） |
| `src/components/SessionList.vue:147` | **已有** `<p v-if="store.error">` 渲染位 —— 只要 store 记录错误即可复用 |
| `src/App.vue:93` | 队列刷新失败仅 `console.error`，面板显示与「无任务」相同的空状态 |
| `src/App.vue:230` | 传输事件订阅失败仅 `console.error`，此后队列永不更新 |

---

### Task 8: sessions store 记录断开/删除错误

**Files:**
- Create: `tests/unit/sessionsStore.spec.ts`
- Modify: `src/stores/sessions.ts`（`disconnect`、`deleteSessionById`）

**Interfaces:**
- Consumes: `disconnectSession`、`deleteSession`、`listSessions`（`src/ipc/client.ts`）。
- Produces: 失败时 `store.error` 非空且 Promise 照常 reject（`App.vue:187` 的 `ElMessage.error` 路径不受影响）。

- [ ] **Step 1: 写失败测试**

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const disconnectSession = vi.fn();
const deleteSession = vi.fn();
const listSessions = vi.fn().mockResolvedValue([]);

vi.mock("../../src/ipc/client", () => ({
  listSessions: (...a: unknown[]) => listSessions(...a),
  createSession: vi.fn(),
  connectSession: vi.fn(),
  disconnectSession: (...a: unknown[]) => disconnectSession(...a),
  deleteSession: (...a: unknown[]) => deleteSession(...a),
}));
vi.mock("../../src/ipc/events", () => ({
  subscribeAppEvents: vi.fn(async () => () => {}),
}));

const { useSessionsStore } = await import("../../src/stores/sessions");

describe("sessions store error surfacing", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    disconnectSession.mockReset();
    deleteSession.mockReset();
    listSessions.mockResolvedValue([]);
  });

  it("records the disconnect failure so the session list can render it", async () => {
    const store = useSessionsStore();
    disconnectSession.mockRejectedValueOnce("backend refused");

    await expect(store.disconnect("session-1")).rejects.toThrow();
    expect(store.error).toContain("backend refused");
  });

  it("records the delete failure instead of failing silently", async () => {
    const store = useSessionsStore();
    deleteSession.mockRejectedValueOnce("not found");

    await expect(store.delete("session-1")).rejects.toThrow();
    expect(store.error).toContain("not found");
  });

  it("clears the previous error after a successful disconnect", async () => {
    const store = useSessionsStore();
    disconnectSession.mockRejectedValueOnce("boom");
    await store.disconnect("session-1").catch(() => {});
    expect(store.error).toContain("boom");

    disconnectSession.mockResolvedValueOnce(undefined);
    await store.disconnect("session-1");
    expect(store.error).toBeNull();
    expect(store.connectionState.get("session-1")).toBe("disconnected");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -- sessionsStore`
Expected: FAIL —— 前两个用例 `store.error` 为 null（当前代码不记录）

- [ ] **Step 3: 实现**

`src/stores/sessions.ts` 中：

```ts
  async function disconnect(id: Uuid) {
    try {
      await disconnectSession(id);
    } catch (e) {
      error.value = String(e);
      throw e;
    }
    error.value = null;
    connectionState.value.set(id, "disconnected");
    connectionState.value = new Map(connectionState.value);
  }

  async function deleteSessionById(id: Uuid) {
    try {
      await deleteSession(id);
    } catch (e) {
      error.value = String(e);
      throw e;
    }
    error.value = null;
    await refresh();
  }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test -- sessionsStore`
Expected: PASS，3 项

- [ ] **Step 5: 提交（等用户确认）**

建议提交信息： `fix(sessions): surface disconnect and delete failures to the session list`

---

### Task 9: 传输队列加载错误可见

**Files:**
- Create: `tests/unit/TransferPanel.spec.ts`
- Modify: `src/components/TransferPanel.vue`（props + 错误行 + 样式）
- Modify: `src/App.vue`（`transferLoadError` 状态、`refreshTransfers`、订阅失败通知、`:error` 绑定）

**Interfaces:**
- Consumes: 无外部新依赖。
- Produces: `TransferPanel` 新增可选 prop `error?: string | null`；`App.vue` 内 `transferLoadError: Ref<string | null>`。

- [ ] **Step 1: 写失败测试**

```ts
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TransferPanel from "../../src/components/TransferPanel.vue";

const tableStub = {
  props: ["data", "emptyText"],
  template: "<div data-stub=\"table\">{{ data.length }}</div>",
};

function panel(props: Record<string, unknown> = {}) {
  return mount(TransferPanel, {
    props: { expanded: true, items: [], ...props },
    global: { stubs: { "el-table": tableStub, "el-table-column": true } },
  });
}

describe("TransferPanel", () => {
  it("shows an alert instead of a silent empty state when the queue failed to load", () => {
    const wrapper = panel({ error: "无法读取传输队列: ipc failed" });
    const alert = wrapper.find('[role="alert"]');
    expect(alert.exists()).toBe(true);
    expect(alert.text()).toContain("无法读取传输队列");
  });

  it("renders no alert when the queue loads cleanly", () => {
    expect(panel().find('[role="alert"]').exists()).toBe(false);
  });

  it("still renders the empty text for a clean empty queue", () => {
    expect(panel().text()).toContain("暂无传输任务");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test -- TransferPanel`
Expected: FAIL —— `role="alert"` 不存在（当前无错误行）

- [ ] **Step 3: 实现 TransferPanel**

3a. props 增加一行：

```ts
const props = defineProps<{
  expanded: boolean;
  items: TransferItem[];
  /** 队列读取失败时的提示；非空时优先于空状态展示 */
  error?: string | null;
  /** 队列高度,折叠后不占空间 */
  height?: number;
}>();
```

3b. 在 `</header>`（`TransferPanel.vue:129`）之后、`<div v-if="expanded && tab === 'transfer'" class="panel-body">` 之前插入：

```html
    <p v-if="error" class="panel-load-error" role="alert">
      传输队列读取失败：{{ error }}（下表可能不是最新状态）
    </p>
```

3c. `<style scoped>` 追加：

```css
.panel-load-error {
  margin: 6px 10px 0;
  padding: 6px 8px;
  border: 1px solid var(--rs-danger, #f56c6c);
  border-radius: 4px;
  color: var(--rs-danger, #f56c6c);
  font-size: 12px;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test -- TransferPanel`
Expected: PASS，3 项

- [ ] **Step 5: 实现 App.vue 状态**

5a. 在 `const unlistenTransfers` / `mounted` 声明旁加：

```ts
const transferLoadError = ref<string | null>(null);
```

5b. 替换 `refreshTransfers`（`App.vue:91-94`）：

```ts
async function refreshTransfers() {
  try {
    transferItems.value = (await listTransfers()).map(toTransferItem);
    transferLoadError.value = null;
  } catch (error) {
    // 不能静默：面板必须能区分「读取失败」与「确实没有任务」
    transferLoadError.value = String(error);
    console.error("无法读取传输队列", error);
  }
}
```

5c. 订阅失败分支（`App.vue:230`）改为同时通知用户：

```ts
  } catch (error) {
    console.error("无法订阅传输队列", error);
    transferLoadError.value = String(error);
    ElNotification.error({
      title: "传输事件订阅失败",
      message: "传输进度与结果将不再自动刷新，请重启应用。",
    });
  }
```

5d. 模板绑定（`App.vue:353-357`）：

```html
            <TransferPanel
              :expanded="transferPanelExpanded"
              :items="transferItems"
              :error="transferLoadError"
              @toggle="transferPanelExpanded = !transferPanelExpanded"
            />
```

- [ ] **Step 6: 类型检查 + 全量前端测试**

Run: `npm run typecheck && npm test`
Expected: 均通过

- [ ] **Step 7: 提交（等用户确认）**

建议提交信息： `fix(transfer): distinguish queue load failure from an empty queue`

---

# 缺陷 #4：pause/resume 不可达状态未记录

### Task 10: 文档记录 pause/resume 无界面入口

**Files:**
- Modify: `docs/08-incomplete-features.md:21`（「核心接口与界面边界」段落末尾）

**Interfaces:**
- Consumes: 现有文档体例（区分「核心接口」与「界面边界」）。
- Produces: 无代码影响。

- [ ] **Step 1: 追加一句**

在 `docs/08-incomplete-features.md` 的「核心接口与界面边界」段（第 21 行，「主题选择和传输队列不承诺跨进程恢复。」所在段）末尾追加：

```markdown
传输 pause/resume 的核心接口与命令已实现并注册（`PauseTransfer`/`ResumeTransfer`），本轮不提供界面入口，`paused` 状态在界面上不可达；不以占位按钮冒充实现。
```

- [ ] **Step 2: 跑文档契约**

Run: `npm run check:docs`
Expected: `Documentation contract passed (14 current documents; dated historical records excluded).`
（该脚本只检查 `README/CLAUDE/CONTRIBUTING/CHANGELOG/scripts/README` 与 `docs/0\d-*.md`，本句不含 `RDP`、不含失效相对链接，可通过。）

- [ ] **Step 3: 提交（等用户确认）**

建议提交信息： `docs: record pause/resume core api without ui entry`

---

# 收尾

### Task 11: 全量验证与验证记录更新

**Files:**
- Modify: `docs/09-macos-validation.md`（「自动检查」段的测试计数、「真实环境验收清单」不变）

**Interfaces:**
- Consumes: 前面所有任务的产出。
- Produces: 与实际执行一致的验证记录。

- [ ] **Step 1: 跑完整验证清单**

```bash
npm run typecheck
npm test
npm run test:scripts
npm run build
npm run check:docs
cd src-tauri
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo fmt --check
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo clippy --workspace --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test --workspace
```

Expected: 全部通过。若任一失败，先修再重跑，**不得**删除或跳过失败用例。

- [ ] **Step 2: 更新 docs/09 的计数**

把 `docs/09-macos-validation.md:14`（`npm test` 行）与 `:20`（`cargo test --workspace` 行）的数字改成 Step 1 实际输出（新增测试后计数必然上升）；如测试文件数变化，同步 `:14` 的「13 个文件」。

- [ ] **Step 3: 在 docs/09 记录本轮修复结论（不改验收勾选状态）**

在 `docs/09-macos-validation.md` 的「独立审查发现的…」那一段（第 28 段）末尾追加：

```markdown
本轮另修复：SFTP 传输进度改为分块回调并按 200ms 节流广播，速度随 `TransferTaskInfo` 快照下发；删除不可达的 `HostKeyManager::check_host_key` 并对 nil `decision_id` 决策事件做前端防御；断开/删除会话与传输队列读取失败不再静默；pause/resume 核心接口无界面入口已在功能与限制中记录。真实 SSH/SFTP 服务器上的进度观感、面板错误展示等 GUI 项仍属未执行验收，不因单元测试通过而勾选。
```

- [ ] **Step 4: 复跑文档契约并确认工作区状态**

Run: `npm run check:docs && git status --short`
Expected: 文档契约通过；变更文件与本计划的「文件结构」表一致，无多余产物。

- [ ] **Step 5: 提交（等用户确认）**

建议提交信息： `docs: refresh validation record for issue fixes`

---

## 明确不在本计划范围内

- 真实 SSH/SFTP 服务器、物理串口、签名公证的验收（`docs/09` 中相应条目保持未勾选）。
- 传输取消（`cancel_transfer` 只改状态，不会中断进行中的分块拷贝）——本次不改，避免范围蔓延。
- pause/resume 的界面入口（用户已确认保留后端、仅做文档记录）。
- `Authenticating`/`Disconnecting` 两个 Rust 侧未发布状态的前端建模——当前后端不发布，无实际影响；若后续启用需同步 `src/stores/sessions.ts:19` 的联合类型与 `WorkspaceToolbar.vue:156` 的按钮判定。
- 事件负载 `speed_bps` 的直接消费——速度走 `TransferTaskInfo` 快照，事件仅用于触发刷新。
