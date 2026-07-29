# RShell 测试策略文档

> **文档版本**：v1.0  
> **编写日期**：2026-07-29  
> **项目名称**：RShell  
> **关联文档**：`docs/03-detailed-design.md`、`docs/05-development-standards.md`

---

## 1. 测试层次划分

### 1.1 测试金字塔

```
                    ┌───────────┐
                    │  E2E 测试  │  ← 少量，验证关键用户流程
                   ─┤  (5%)     ├─
                  ┌─┴───────────┴─┐
                  │  集成测试       │  ← 验证模块间协作
                 ─┤  (20%)        ├─
                ┌─┴───────────────┴─┐
                │   单元测试         │  ← 大量，覆盖核心逻辑
               ─┤   (75%)           ├─
              ┌─┴───────────────────┴─┐
              │    静态分析 / Lint      │  ← 编译期保障
             ─┴───────────────────────┴─
```

| 层次 | 范围 | 运行速度 | 数量目标 |
|------|------|----------|----------|
| 静态分析 | 编译期类型检查、clippy | 即时 | 0 warning |
| 单元测试 | 单个函数/结构体 | < 100ms/个 | 500+ |
| 集成测试 | 模块间交互 | < 1s/个 | 100+ |
| E2E 测试 | 完整用户流程 | < 30s/个 | 20+ |

### 1.2 各层次定义

| 层次 | 定义 | 工具 | 位置 |
|------|------|------|------|
| 单元测试 | 测试单个函数/方法，隔离外部依赖 | `cargo test` + `mockall` | 各 crate `tests.rs` 或 `#[cfg(test)]` |
| 集成测试 | 测试多个模块协作 | `cargo test --test` | `tests/` 目录 |
| E2E 测试 | 模拟用户操作验证完整流程 | 自定义 harness + 虚拟服务器 | `tests/e2e/` |
| 性能测试 | 验证性能指标不退化 | `criterion` | `benches/` |

---

## 2. 各模块测试重点与覆盖率目标

### 2.1 覆盖率目标

| Crate | 覆盖率目标 | 说明 |
|-------|-----------|------|
| `rshell-api` | 90% | 纯数据类型，主要测试序列化 |
| `rshell-core` | 80% | 核心业务逻辑，重点覆盖 |
| `rshell-protocol` | 75% | 协议实现，含网络交互 |
| `rshell-infra` | 80% | 基础设施，加密/存储 |
| `rshell-plugin-sdk` | 70% | 插件加载/沙箱 |
| `rshell-ui` | 50% | ViewModel 逻辑，UI 交互较难测试 |
| **整体** | **≥ 70%** | |

### 2.2 各模块测试重点

#### 终端模块 (`core-terminal`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| VT 序列解析正确性 | 单元 | P0 |
| 缓冲区操作（插入/删除/滚动） | 单元 | P0 |
| 终端 resize 处理 | 单元 | P0 |
| PTY 创建/销毁生命周期 | 集成 | P0 |
| 字符编码（UTF-8/CJK 宽度） | 单元 | P1 |
| 终端快照序列化 | 单元 | P1 |
| 快速输出性能 | 基准 | P1 |

```rust
// 示例：VT 序列解析测试
#[test]
fn test_csi_cursor_move() {
    let mut parser = VtParser::new(24, 80);
    parser.feed("\x1b[10;20H");  // CUP 光标移动
    let cursor = parser.cursor();
    assert_eq!(cursor.row, 9);   // 0-based
    assert_eq!(cursor.col, 19);
}

// 示例：缓冲区滚动测试
#[test]
fn test_scroll_up_with_history() {
    let mut buffer = TerminalBuffer::new(24, 80, 1000);
    for i in 0..30 {
        buffer.write_line(&format!("Line {}", i));
    }
    assert_eq!(buffer.visible_lines()[0].text(), "Line 6");
    assert_eq!(buffer.history().len(), 6);
}
```

#### SSH 协议模块 (`protocol-ssh`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| 密码认证流程 | 集成 | P0 |
| 公钥认证流程（RSA/Ed25519） | 集成 | P0 |
| 通道管理（Session/Channel） | 集成 | P0 |
| 密钥交换算法协商 | 单元 | P1 |
| 连接超时/重连 | 集成 | P1 |
| 主机密钥验证 | 单元 | P0 |

```rust
// 示例：SSH 连接测试（使用 mock server）
#[tokio::test]
async fn test_ssh_password_auth() {
    let mock_server = MockSshServer::start().await;
    let config = SessionConfig {
        host: "127.0.0.1".into(),
        port: mock_server.port(),
        auth: AuthMethod::Password { password: "test123".into() },
        ..Default::default()
    };
    let conn = SshConnection::connect(&config).await.unwrap();
    assert!(conn.is_authenticated());
}
```

#### Telnet 协议模块 (`protocol-telnet`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| Telnet 选项协商（WILL/WONT/DO/DONT） | 单元 | P0 |
| 终端类型协商（TerminalType） | 单元 | P1 |
| 窗口大小协商（WindowSize） | 单元 | P1 |
| 连接生命周期 | 集成 | P0 |

#### Serial 串口模块 (`protocol-serial`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| 串口配置（波特率/数据位/停止位） | 单元 | P0 |
| 串口打开/关闭 | 集成 | P0 |
| 数据读写 | 集成 | P1 |

#### RDP 远程桌面模块 (`protocol-rdp`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| RDP 连接建立 | 集成 | P1 |
| 输入事件转发 | 集成 | P1 |
| 画面渲染集成 | 集成 | P1 |

#### SFTP 文件传输模块 (`core-transfer`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| 目录列表解析 | 单元 | P0 |
| 文件上传/下载 | 集成 | P0 |
| 断点续传逻辑 | 集成 | P0 |
| 传输队列管理（暂停/恢复/取消） | 单元 | P1 |
| 大文件传输内存控制 | 基准 | P1 |
| 文件夹同步差异计算 | 单元 | P1 |

```rust
// 示例：断点续传测试
#[tokio::test]
async fn test_resume_interrupted_upload() {
    let mut transfer = FileTransfer::new(local_path, remote_path);
    // 模拟已传输 50%
    transfer.set_progress(500, 1000);
    transfer.resume().await.unwrap();
    assert_eq!(transfer.progress().bytes_transferred, 1000);
}
```

#### 会话管理模块 (`core-session`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| 会话 CRUD | 单元 | P0 |
| 会话持久化（TOML 序列化/反序列化） | 单元 | P0 |
| 属性继承 | 单元 | P1 |
| 会话树结构 | 单元 | P1 |
| Xshell .xsh 导入解析 | 单元 | P2 |

#### 安全模块 (`core-security`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| 密钥生成（RSA/Ed25519/ECDSA） | 单元 | P0 |
| 密钥导入/导出格式 | 单元 | P0 |
| 主密码加密/解密 | 单元 | P0 |
| 隧道创建（本地/远程/动态） | 集成 | P0 |
| 主机密钥指纹验证 | 单元 | P1 |

#### 脚本模块 (`core-script`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| 快速命令执行 | 单元 | P0 |
| 触发器模式匹配 | 单元 | P0 |
| 触发器动作执行 | 集成 | P1 |
| Rhai 脚本执行 | 集成 | P1 |
| 脚本录制/回放 | 集成 | P2 |

#### 插件模块 (`rshell-plugin-sdk`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| 插件清单解析 | 单元 | P0 |
| WASM 插件加载/卸载 | 集成 | P0 |
| 插件沙箱隔离 | 集成 | P0 |
| 权限控制 | 单元 | P0 |
| 扩展点注册/调用 | 集成 | P1 |

#### UI 模块 (`rshell-ui`)

| 测试重点 | 类型 | 优先级 |
|----------|------|--------|
| ViewModel 状态更新 | 单元 | P0 |
| Command 构造正确性 | 单元 | P1 |
| Event → ViewModel 映射 | 单元 | P1 |
| 配置序列化 | 单元 | P1 |

---

## 3. 测试工具选型

### 3.1 工具矩阵

| 工具 | 用途 | 版本 | 所属 |
|------|------|------|------|
| `cargo test` | 单元测试/集成测试框架 | 内置 | Rust 标准 |
| `mockall` | Mock 对象生成 | 0.13+ | dev-dependency |
| `criterion` | 性能基准测试 | 0.5+ | dev-dependency |
| `tokio-test` | 异步代码测试辅助 | 最新 | dev-dependency |
| `tempfile` | 临时文件/目录（测试用） | 3.x | dev-dependency |
| `assert_cmd` | CLI 集成测试 | 2.x | dev-dependency |
| `cargo-tarpaulin` | 代码覆盖率 | 最新 | CI 工具 |
| `cargo-deny` | 许可证/安全审计 | 最新 | CI 工具 |
| `wiremock` | HTTP mock（插件市场） | 0.6+ | dev-dependency |

### 3.2 Mock 策略

```rust
// 使用 mockall 为 Service trait 生成 Mock
use mockall::*;

#[automock]
pub trait TerminalService: Send + Sync {
    fn create_terminal(&self, config: TerminalConfig) -> Result<Uuid>;
    async fn send_input(&self, terminal_id: Uuid, data: &[u8]) -> Result<()>;
    fn get_buffer_snapshot(&self, terminal_id: Uuid) -> Result<TerminalBufferSnapshot>;
}

// 在 UI 测试中使用 Mock
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_view_model_updates_on_event() {
        let mut vm = TerminalViewModel::new();
        let event = AppEvent::TerminalOutput {
            terminal_id: Uuid::new_v4(),
            snapshot: TerminalBufferSnapshot::default(),
        };
        vm.handle_event(&event);
        assert_eq!(vm.buffer.rows, 24);
    }
}
```

### 3.3 测试数据管理

```
tests/
├── fixtures/              # 测试数据
│   ├── sessions/          # 会话配置样本
│   │   ├── basic.toml
│   │   ├── with_proxy.toml
│   │   └── xshell_compat.xsh
│   ├── keys/              # 测试用密钥（非真实密钥）
│   │   ├── rsa_test_key
│   │   └── ed25519_test_key
│   ├── sftp/              # SFTP 测试文件
│   │   ├── small.txt      # 1KB
│   │   ├── medium.bin     # 1MB
│   │   └── large.bin      # 100MB（git-lfs）
│   └── plugins/           # 测试插件
│       └── sample_plugin/
└── e2e/                   # E2E 测试
    ├── test_ssh_connect.rs
    └── test_file_transfer.rs
```

---

## 4. 自动化测试流水线设计

### 4.1 CI 流水线（GitHub Actions）

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  # Job 1: 静态分析
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all -- -D warnings
      - run: cargo deny check

  # Job 2: 单元测试 + 覆盖率
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-14, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace --all-features
      # 仅 Linux 收集覆盖率
      - if: matrix.os == 'ubuntu-latest'
        run: |
          cargo install cargo-tarpaulin
          cargo tarpaulin --workspace --out xml
      - if: matrix.os == 'ubuntu-latest'
        uses: codecov/codecov-action@v4
        with:
          file: cobertura.xml

  # Job 3: 集成测试（需要测试服务器）
  integration:
    runs-on: ubuntu-latest
    services:
      ssh-server:
        image: linuxserver/openssh-server
        ports: ['2222:2222']
      sftp-server:
        image: atmoz/sftp
        ports: ['2223:22']
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --test '*' -- --ignored

  # Job 4: 性能基准（仅 main 分支）
  bench:
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo bench --workspace
      - uses: actions/upload-artifact@v4
        with:
          name: bench-results
          path: target/criterion/
```

### 4.2 本地测试流程

```bash
# 开发者每次提交前执行
#!/bin/bash
set -e

echo "=== 格式化检查 ==="
cargo fmt --all -- --check

echo "=== Clippy 检查 ==="
cargo clippy --all -- -D warnings

echo "=== 单元测试 ==="
cargo test --workspace

echo "=== 集成测试 ==="
cargo test --test '*'

echo "✅ 所有检查通过"
```

### 4.3 Pre-commit Hook

```bash
# .githooks/pre-commit
#!/bin/bash
cargo fmt --all -- --check || { echo "cargo fmt 检查失败"; exit 1; }
cargo clippy --all -- -D warnings || { echo "clippy 检查失败"; exit 1; }
cargo test --workspace --lib || { echo "单元测试失败"; exit 1; }
```

---

## 5. 性能测试与压力测试方案

### 5.1 性能基准测试

```rust
// benches/terminal_render_bench.rs
use criterion::{criterion_group, criterion_main, Criterion};

fn terminal_render_benchmark(c: &mut Criterion) {
    let mut buffer = TerminalBuffer::new(50, 120, 10000);
    // 填充测试数据
    for i in 0..5000 {
        buffer.write_line(&format!("\x1b[31mError\x1b[0m: Line {}", i));
    }

    c.bench_function("terminal_snapshot_50x120", |b| {
        b.iter(|| buffer.snapshot());
    });
}

criterion_group!(benches, terminal_render_benchmark);
criterion_main!(benches);
```

### 5.2 压力测试场景

| 场景 | 方法 | 通过标准 |
|------|------|----------|
| 多标签并发输出 | 开 20 个 SSH 会话同时 `cat /dev/urandom \| base64` | 无卡顿，FPS ≥ 30 |
| 大文件传输 | 传输 1GB 文件 | 内存 < 100MB，无泄漏 |
| 长时间运行 | 保持 10 个会话 24 小时 | 内存增长 < 10% |
| 快速切换标签 | 100 次/秒切换 | 无崩溃，无渲染错误 |
| 大量会话 | 开 50 个标签 | 内存 < 500MB |
| 历史缓冲区极限 | 单终端 100000 行历史 | 滚动流畅 |

### 5.3 内存泄漏检测

```bash
# 使用 valgrind (Linux) 检测内存泄漏
cargo build --release
valgrind --leak-check=full --show-leak-kinds=all \
    ./target/release/rshell --test-mode

# 使用 Address Sanitizer (跨平台)
RUSTFLAGS="-Z sanitizer=address" cargo test --workspace
```

### 5.4 测试环境配置

| 环境 | 配置 | 用途 |
|------|------|------|
| 本地开发 | 本机 + Docker SSH | 日常开发测试 |
| CI | GitHub Actions + Docker services | 自动化集成测试 |
| 性能测试 | 专用 Linux 机器 | 基准测试、压力测试 |

```yaml
# docker-compose.yml（测试用）
version: '3.8'
services:
  ssh-server:
    image: linuxserver/openssh-server
    ports:
      - "2222:2222"
    environment:
      - PASSWORD_ACCESS=true
      - USER_PASSWORD=test123
  sftp-server:
    image: atmoz/sftp
    ports:
      - "2223:22"
    command: testuser:test123:1001
  ftp-server:
    image: fauria/vsftpd
    ports:
      - "21:21"
```

---

## 附录

### A. 测试覆盖率报告模板

```
┌─────────────────────┬──────────┬──────────┬──────────┐
│ Crate               │ 行覆盖率  │ 函数覆盖率 │ 分支覆盖率 │
├─────────────────────┼──────────┼──────────┼──────────┤
│ rshell-api          │   92%    │   95%    │   88%    │
│ rshell-core         │   81%    │   85%    │   72%    │
│ rshell-protocol     │   76%    │   80%    │   65%    │
│ rshell-infra        │   83%    │   87%    │   74%    │
│ rshell-plugin-sdk   │   71%    │   75%    │   60%    │
│ rshell-ui           │   52%    │   58%    │   40%    │
├─────────────────────┼──────────┼──────────┼──────────┤
│ 总计                 │   75%    │   80%    │   66%    │
└─────────────────────┴──────────┴──────────┴──────────┘
```

### B. Bug 报告模板

```markdown
## Bug 描述
[清晰简洁的描述]

## 复现步骤
1. ...
2. ...
3. ...

## 期望行为
[期望发生什么]

## 实际行为
[实际发生了什么]

## 环境信息
- OS: [Windows 11 / macOS 14 / Ubuntu 22.04]
- RShell 版本: [v0.1.0]
- Rust 版本: [1.80.0]

## 日志
[相关日志输出]

## 截图
[如果适用]
```
