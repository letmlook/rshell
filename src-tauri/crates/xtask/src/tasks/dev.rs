//! `cargo xtask dev` — 启动 Tauri 开发环境（转发 `npm run tauri:dev`）

use std::path::PathBuf;
use std::process::Command;

pub fn run() -> Result<(), String> {
    // R2-19：旧实现指向的 UI 包已随 RDP/UI 重构删除（见 git 历史）。开发环境
    // 的权威入口是 CLAUDE.md 规定的 `npm run tauri:dev`（在仓库根目录执行，
    // package.json 在根）。
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // src-tauri/crates/xtask
    let repo_root = manifest_dir
        .ancestors()
        .nth(3)
        .ok_or("cannot locate repository root from xtask manifest")?
        .to_path_buf();

    let status = Command::new("npm")
        .args(["run", "tauri:dev"])
        .current_dir(&repo_root)
        .status()
        .map_err(|e| format!("failed to spawn npm (cwd {}): {}", repo_root.display(), e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("npm exited with status {}", status))
    }
}
