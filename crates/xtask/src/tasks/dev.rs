//! `cargo xtask dev` — 启动 rshell-ui 开发版本

use crate::cargo;

pub fn run() -> Result<(), String> {
    cargo(["run", "-p", "rshell-ui"])
}