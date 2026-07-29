# RShell 项目初始化指南

> **文档版本**：v1.0  
> **编写日期**：2026-07-29  
> **项目名称**：RShell  
> **关联文档**：`docs/03-detailed-design.md`

---

## 1. Cargo Workspace 目录结构

### 1.1 总体结构

```
rshell/
├── Cargo.toml                    # Workspace 根配置
├── Cargo.lock
├── rust-toolchain.toml           # Rust 工具链版本锁定
├── deny.toml                     # cargo-deny 配置
├── .cargo/
│   └── config.toml               # Cargo 全局配置
├── .github/
│   └── workflows/
│       └── ci.yml                # GitHub Actions CI
├── .githooks/
│   └── pre-commit                # Git pre-commit hook
├── docs/                         # 项目文档
│   ├── 01-xshell-xftp-feature-research.md
│   ├── 02-project-plan.md
│   ├── 03-detailed-design.md
│   ├── 04-technical-feasibility.md
│   ├── 05-development-standards.md
│   ├── 06-test-strategy.md
│   └── 07-project-setup-guide.md
│
├── crates/                       # 所有 crate 源码
│   ├── rshell-api/               # API 边界层（零依赖）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── commands.rs       # Command 定义
│   │       └── events.rs         # Event 定义
│   │
│   ├── rshell-ui/                # 前端层（GPUI）
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs           # 应用入口
│   │       ├── app.rs            # 应用根组件
│   │       ├── view_models/      # ViewModel 层
│   │       └── views/            # View 组件
│   │
│   ├── rshell-core/              # 后端层
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── terminal/         # 终端服务
│   │       ├── session/          # 会话管理
│   │       ├── transfer/         # 文件传输
│   │       ├── security/         # 安全服务
│   │       └── script/           # 脚本引擎
│   │
│   ├── rshell-protocol/          # 协议层
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ssh/              # SSH 协议
│   │       ├── telnet/           # Telnet 协议
│   │       ├── serial/           # 串口协议
│   │       └── rdp/              # RDP 协议
│   │
│   ├── rshell-infra/             # 基础设施层
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── crypto/           # 加密
│   │       ├── storage/          # 持久化
│   │       └── pty/              # PTY 抽象
│   │
│   └── rshell-plugin-sdk/        # 插件 SDK
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── loader.rs         # 插件加载器
│           ├── sandbox.rs        # WASM 沙箱
│           └── api.rs            # 插件 API
│
├── tests/                        # 集成测试
│   ├── fixtures/                 # 测试数据
│   └── e2e/                      # E2E 测试
│
├── benches/                      # 基准测试
│   └── terminal_bench.rs
│
└── docker-compose.yml            # 测试环境
```

### 1.2 Workspace 根 Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
    "crates/rshell-api",
    "crates/rshell-ui",
    "crates/rshell-core",
    "crates/rshell-protocol",
    "crates/rshell-infra",
    "crates/rshell-plugin-sdk",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
license = "Apache-2.0"
repository = "https://github.com/your-org/rshell"

[workspace.dependencies]
# 异步运行时
tokio = { version = "1", features = ["full"] }

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# 错误处理
thiserror = "2"
anyhow = "1"

# 日志
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# 工具
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
regex = "1"

# 测试
mockall = "0.13"
criterion = { version = "0.5", features = ["html_reports"] }
tempfile = "3"
tokio-test = "0.4"

# 内部 crate 依赖（路径引用）
rshell-api = { path = "crates/rshell-api" }
rshell-core = { path = "crates/rshell-core" }
rshell-protocol = { path = "crates/rshell-protocol" }
rshell-infra = { path = "crates/rshell-infra" }
rshell-plugin-sdk = { path = "crates/rshell-plugin-sdk" }

[profile.release]
lto = true
codegen-units = 1
strip = true

[profile.dev]
opt-level = 0
debug = true
```

---

## 2. 各 Crate 初始 Cargo.toml

### 2.1 rshell-api

```toml
# crates/rshell-api/Cargo.toml
[package]
name = "rshell-api"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "RShell API 边界层 - Command/Event 定义（零运行时依赖）"

[dependencies]
serde = { workspace = true }
uuid = { workspace = true }
# 注意：不依赖 GPUI、tokio 或其他运行时
```

### 2.2 rshell-infra

```toml
# crates/rshell-infra/Cargo.toml
[package]
name = "rshell-infra"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "RShell 基础设施层 - 加密、存储、PTY 抽象"

[dependencies]
rshell-api = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
toml = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
ring = "0.17"
dirs = "5"

[dev-dependencies]
tempfile = { workspace = true }
mockall = { workspace = true }
```

### 2.3 rshell-protocol

```toml
# crates/rshell-protocol/Cargo.toml
[package]
name = "rshell-protocol"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "RShell 协议层 - SSH/Telnet/Serial/RDP 协议实现"

[dependencies]
rshell-api = { workspace = true }
rshell-infra = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
russh = "0.48"
russh-keys = "0.48"
russh-sftp = "0.14"

[dev-dependencies]
tokio-test = { workspace = true }
tempfile = { workspace = true }
```

### 2.4 rshell-core

```toml
# crates/rshell-core/Cargo.toml
[package]
name = "rshell-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "RShell 后端层 - 终端、会话、传输、安全、脚本服务"

[dependencies]
rshell-api = { workspace = true }
rshell-protocol = { workspace = true }
rshell-infra = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
alacritty_terminal = "0.24"
rhai = "1"

[dev-dependencies]
mockall = { workspace = true }
tokio-test = { workspace = true }
tempfile = { workspace = true }
```

### 2.5 rshell-plugin-sdk

```toml
# crates/rshell-plugin-sdk/Cargo.toml
[package]
name = "rshell-plugin-sdk"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "RShell 插件 SDK - 插件加载、沙箱、扩展点"

[dependencies]
rshell-api = { workspace = true }
rshell-core = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
wasmtime = "28"
libloading = "0.8"
sha2 = "0.10"

[dev-dependencies]
tempfile = { workspace = true }
```

### 2.6 rshell-ui

```toml
# crates/rshell-ui/Cargo.toml
[package]
name = "rshell-ui"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "RShell 前端层 - GPUI 视图、ViewModel、应用入口"

[dependencies]
rshell-api = { workspace = true }
# 注意：不直接依赖 rshell-core / rshell-protocol
serde = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uuid = { workspace = true }
tokio = { workspace = true }
gpui = "0.2"
gpui-component = "0.5"

[dev-dependencies]
mockall = { workspace = true }
```

---

## 3. 开发环境搭建

### 3.1 系统要求

| 项目 | 最低要求 | 推荐 |
|------|----------|------|
| Rust | 1.80+ (stable) | 最新 stable |
| OS | Windows 10 / macOS 12 / Ubuntu 22.04 | Windows 11 / macOS 14 |
| 内存 | 8GB | 16GB+ |
| 磁盘 | 10GB 可用空间 | 20GB+ SSD |
| GPU | 支持 DirectX 11 / Metal / Vulkan | 独显 |

### 3.2 Rust 工具链安装

```bash
# 1. 安装 rustup（如未安装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Windows (PowerShell):
Invoke-WebRequest -Uri https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe -OutFile rustup-init.exe
.\rustup-init.exe

# 2. 设置工具链版本
rustup toolchain install 1.80
rustup default stable

# 3. 安装必要组件
rustup component add rustfmt clippy

# 4. 验证安装
rustc --version     # rustc 1.80.0+
cargo --version     # cargo 1.80.0+
```

### 3.3 系统依赖

#### Windows

```powershell
# 安装 Visual Studio Build Tools（含 C++ 工具链）
# 下载: https://visualstudio.microsoft.com/visual-cpp-build-tools/
# 勾选: "使用 C++ 的桌面开发"

# 安装 Vulkan SDK（GPUI 渲染需要）
# 下载: https://vulkan.lunarg.com/sdk/home

# 安装 CMake（部分 crate 编译需要）
winget install Kitware.CMake
```

#### macOS

```bash
# 安装 Xcode Command Line Tools
xcode-select --install

# 安装 Homebrew 依赖
brew install cmake pkg-config
```

#### Linux (Ubuntu)

```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    libfontconfig1-dev \
    libfreetype6-dev \
    libexpat1-dev \
    cmake \
    libvulkan-dev \
    libx11-dev \
    libxkbcommon-dev \
    libwayland-dev
```

### 3.4 开发工具安装

```bash
# Cargo 扩展工具
cargo install cargo-watch          # 文件变更自动重编译
cargo install cargo-deny           # 许可证/安全审计
cargo install cargo-tarpaulin      # 代码覆盖率（Linux）
cargo install cargo-nextest        # 更快的测试运行器

# IDE（推荐）
# - VS Code + rust-analyzer 扩展
# - 或 RustRover (JetBrains)
```

### 3.5 项目初始化步骤

```bash
# 1. 克隆仓库
git clone https://github.com/your-org/rshell.git
cd rshell

# 2. 创建目录结构
mkdir -p crates/rshell-api/src
mkdir -p crates/rshell-ui/src/view_models
mkdir -p crates/rshell-ui/src/views
mkdir -p crates/rshell-core/src/{terminal,session,transfer,security,script}
mkdir -p crates/rshell-protocol/src/{ssh,telnet,serial,rdp}
mkdir -p crates/rshell-infra/src/{crypto,storage,pty}
mkdir -p crates/rshell-plugin-sdk/src
mkdir -p tests/fixtures/{sessions,keys,sftp,plugins}
mkdir -p tests/e2e
mkdir -p benches

# 3. 创建 Cargo.toml 文件（按上述各 crate 配置）
# 4. 创建初始 lib.rs / main.rs 文件
# 5. 验证编译
cargo check --workspace

# 6. 设置 Git hooks
git config core.hooksPath .githooks
chmod +x .githooks/pre-commit
```

---

## 4. CI/CD 流水线配置

### 4.1 GitHub Actions CI

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  lint:
    name: Lint & Format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings

  test:
    name: Test (${{ matrix.os }})
    strategy:
      matrix:
        os: [ubuntu-latest, macos-14, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --all-features

  coverage:
    name: Coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo install cargo-tarpaulin
      - run: cargo tarpaulin --workspace --out xml
      - uses: codecov/codecov-action@v4

  integration:
    name: Integration Tests
    runs-on: ubuntu-latest
    services:
      ssh:
        image: linuxserver/openssh-server
        ports: ['2222:2222']
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --test '*' -- --ignored

  deny:
    name: License Audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo install cargo-deny
      - run: cargo deny check

  release:
    name: Release (${{ matrix.target }})
    if: startsWith(github.ref, 'refs/tags/v')
    needs: [lint, test, integration, deny]
    strategy:
      matrix:
        include:
          - target: x86_64-pc-windows-msvc
            os: windows-latest
            ext: .exe
          - target: x86_64-apple-darwin
            os: macos-latest
            ext: ''
          - target: aarch64-apple-darwin
            os: macos-latest
            ext: ''
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
            ext: ''
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release --target ${{ matrix.target }}
      - uses: actions/upload-artifact@v4
        with:
          name: rshell-${{ matrix.target }}
          path: target/${{ matrix.target }}/release/rshell${{ matrix.ext }}
```

### 4.2 rust-toolchain.toml

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.80"
components = ["rustfmt", "clippy"]
```

### 4.3 .cargo/config.toml

```toml
# .cargo/config.toml
[build]
# 并行编译
jobs = 8

[target.x86_64-pc-windows-msvc]
# Windows 链接器优化
rustflags = ["-C", "target-feature=+crt-static"]

[alias]
# 常用命令别名
xtask = "run --package xtask --"
```

### 4.4 deny.toml

```toml
# deny.toml
[advisories]
vulnerability = "deny"
unmaintained = "warn"

[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "ISC",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "MPL-2.0",
    "Unicode-3.0",
    "Unicode-DFS-2016",
]

[bans]
multiple-versions = "warn"
skip = [
    # 允许部分 crate 多版本共存（过渡期）
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

---

## 5. 本地开发调试流程

### 5.1 日常开发命令

```bash
# 启动开发（文件变更自动重编译）
cargo watch -x "run -p rshell-ui"

# 仅检查编译（快速反馈）
cargo check --workspace

# 运行所有测试
cargo test --workspace

# 运行指定 crate 测试
cargo test -p rshell-core

# 运行指定测试函数
cargo test -p rshell-core terminal::tests::test_cursor_move

# 运行基准测试
cargo bench -p rshell-core

# Clippy 检查
cargo clippy --workspace -- -D warnings

# 格式化
cargo fmt --all
```

### 5.2 测试环境启动

```bash
# 启动 Docker 测试环境
docker-compose up -d

# 验证 SSH 测试服务器
ssh -p 2222 testuser@localhost  # 密码: test123

# 验证 SFTP 测试服务器
sftp -P 2223 testuser@localhost

# 停止测试环境
docker-compose down
```

### 5.3 调试技巧

```bash
# 启用详细日志
RUST_LOG=debug cargo run -p rshell-ui

# 启用特定模块日志
RUST_LOG=rshell_core::terminal=trace cargo run -p rshell-ui

# 内存分析（Linux）
RUSTFLAGS="-Z sanitizer=address" cargo test --workspace

# 性能分析
cargo flamegraph --bin rshell
```

### 5.4 开发工作流

```
1. 从 main 拉出功能分支
   git checkout -b feat/xxx

2. 开发 + 本地验证
   cargo check --workspace
   cargo test --workspace
   cargo clippy --workspace -- -D warnings
   cargo fmt --all -- --check

3. 提交（触发 pre-commit hook）
   git commit -m "feat(terminal): add xxx"

4. 推送 + 创建 PR
   git push origin feat/xxx
   # GitHub 上创建 PR → 触发 CI

5. Code Review + CI 通过 → 合并
```

---

## 附录

### A. 常见问题

| 问题 | 解决方案 |
|------|----------|
| GPUI 编译失败 | 确认 Vulkan SDK 已安装，`VULKAN_SDK` 环境变量已设置 |
| russh 编译失败 | 确认 OpenSSL 开发库已安装（`libssl-dev`） |
| Windows 链接错误 | 确认 Visual Studio Build Tools 已安装 C++ 组件 |
| 字体加载失败 | 确认系统已安装等宽字体（Consolas/Menlo/monospace） |
| cargo-deny 报错 | 运行 `cargo deny check --hide-inclusion-graph` 查看详情 |

### B. 快速参考

| 操作 | 命令 |
|------|------|
| 新建 crate | `cargo new crates/rshell-xxx --lib` |
| 添加依赖 | `cargo add -p rshell-core tokio` |
| 更新依赖 | `cargo update` |
| 查看依赖树 | `cargo tree -p rshell-core` |
| 清理构建缓存 | `cargo clean` |
| 查看编译时间 | `cargo build --timings` |
