//! `cargo xtask build` — release 构建

use crate::cargo;

pub fn run() -> Result<(), String> {
    cargo(["build", "--release"])
}