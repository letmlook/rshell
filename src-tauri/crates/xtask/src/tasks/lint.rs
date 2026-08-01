//! `cargo xtask lint` — clippy 全 workspace 扫描，告警视为错误

use crate::cargo;

pub fn run() -> Result<(), String> {
    cargo([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])
}