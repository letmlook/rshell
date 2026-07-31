//! `cargo xtask fmt` — 格式化整个 workspace

use crate::cargo;

pub fn run() -> Result<(), String> {
    cargo(["fmt", "--all"])
}