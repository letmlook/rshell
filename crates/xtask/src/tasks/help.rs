//! `cargo xtask xtask-help` — 打印 xtask 子命令一览

pub fn run() -> Result<(), String> {
    println!("xtask subcommands:");
    println!("  fmt      — cargo fmt --all");
    println!("  lint     — cargo clippy --workspace --all-targets -- -D warnings");
    println!("  test     — cargo test --workspace");
    println!("  dev      — cargo run -p rshell-ui");
    println!("  build    — cargo build --release");
    println!();
    println!("或者直接用 `cargo xtask <subcommand>` 调用。");
    Ok(())
}