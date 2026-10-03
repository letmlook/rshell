//! `cargo xtask xtask-help` — 打印 xtask 子命令一览

pub fn run() -> Result<(), String> {
    println!("xtask subcommands:");
    println!("  fmt      — cargo fmt --all");
    println!("  lint     — cargo clippy --workspace --all-targets -- -D warnings");
    println!("  test     — cargo test --workspace");
    println!("  dev      — npm run tauri:dev（在仓库根目录）");
    println!("  build    — cargo build --release");
    println!();
    println!("或者直接用 `cargo xtask <subcommand>` 调用。");
    Ok(())
}
