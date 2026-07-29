//! `cargo xtask test` — 运行整个 workspace 的测试

use crate::cargo;

pub fn run() -> Result<(), String> {
    cargo(["test", "--workspace"])
}