//! `xtask` — RShell 任务执行器
//!
//! 将常用 cargo workflow 封装为子命令，统一开发流程入口。
//!
//! ```text
//! cargo xtask fmt      # cargo fmt --all
//! cargo xtask lint     # cargo clippy --workspace --all-targets -- -D warnings
//! cargo xtask test     # cargo test --workspace
//! cargo xtask dev      # cargo run -p rshell-ui
//! cargo xtask build    # cargo build --release
//! ```

use clap::{Parser, Subcommand};
use std::process::{Command, ExitCode};

mod tasks;

#[derive(Debug, Parser)]
#[command(name = "xtask", version, about = "RShell task runner")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    /// 格式化整个 workspace
    Fmt,
    /// 运行 clippy，告警视为错误
    Lint,
    /// 运行 workspace 全部测试
    Test,
    /// 启动 rshell-ui 开发版本
    Dev,
    /// release 构建
    Build,
    /// 打印本工具自身的 cargo xtask 解析
    XtaskHelp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        CommandKind::Fmt => tasks::fmt::run(),
        CommandKind::Lint => tasks::lint::run(),
        CommandKind::Test => tasks::test::run(),
        CommandKind::Dev => tasks::dev::run(),
        CommandKind::Build => tasks::build::run(),
        CommandKind::XtaskHelp => tasks::help::run(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// 共享工具：调用 cargo 子进程
fn cargo<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let status = Command::new("cargo")
        .args(args)
        .status()
        .map_err(|e| format!("failed to spawn cargo: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo exited with status {}", status))
    }
}