//! 孤儿 staging 临时文件的识别与回收。
//!
//! staged 传输把数据先写进 `<目标名>.partial-<task_uuid>`，成功后再 rename
//! 覆盖目标。正常路径上临时文件一定会被提交或清理；但进程被强杀 / 断电 /
//! 清理本身失败时，临时文件会**留在磁盘上**——之后再没有任何代码会碰它。
//!
//! 本模块提供两件事：
//!
//! 1. **严格的识别规则**（[`is_reclaimable_partial`]）：只有名字形如
//!    `<目标名>.partial-<合法 uuid>` 的**普通文件**才可能被回收，且必须比
//!    [`PARTIAL_RECLAIM_MIN_AGE`] 更老、且其 task id 不属于本进程的在途任务。
//!    四个条件缺一不可——尤其是「目标名必须与本次传输的目标一致」：
//!    否则用户自己造一个 `backup.partial-<uuid>` 这样的文件名就会被误删。
//! 2. **实际的删除动作**（[`reclaim_local_partials`]）：本地目录扫描。
//!    远端侧的对称实现放在 `transfer::service`（需要 SFTP 客户端）。
//!
//! 回收只在「本进程刚往这个目录写过临时文件」时触发——即传输任务收尾时。
//! 这是刻意的：应用没有持久化「我碰过哪些目录」的账本，无从在启动时
//! 凭空知道该去哪里扫。代价是：目录里再也不会有传输发生时，那次崩溃留下
//! 的孤儿文件会一直留着（失败路径上我们会把确切路径报给用户，见
//! `cleanup_status: residue`）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// staging 临时文件的后缀标记：`<目标名>.partial-<task_uuid>`。
pub const PARTIAL_SUFFIX: &str = ".partial-";

/// 本地孤儿临时文件的最小存留时长。
///
/// 只有 mtime 比这更早的临时文件才可能被回收。取值必须**大于一次正常传输
/// 的耗时**，否则可能删掉另一个 RShell 进程（同一台机器上同时跑着开发版
/// 与安装版很常见）仍在写的临时文件。1 小时对绝大多数传输来说足够宽裕。
pub const PARTIAL_RECLAIM_MIN_AGE: Duration = Duration::from_secs(60 * 60);

/// 远端孤儿临时文件的最小存留时长。
///
/// 远端的 mtime 来自**服务器自己的时钟**，与本地可能差很多（容器宿主、
/// 未同步 NTP 的机器）。若服务器时钟比本地**慢**，对方刚写出来的临时文件在
/// 本地看来已经「超龄」——按 1 小时阈值就会误删另一个实例正在写的同名
/// 临时文件。
///
/// 因此远端用更长的阈值（24 小时），给时钟偏差留出足够余量；本地文件用
/// 本地时钟，1 小时即可。
pub const PARTIAL_RECLAIM_MIN_AGE_REMOTE: Duration = Duration::from_secs(24 * 60 * 60);

/// 从文件名尾部解析 staging 任务 id。
///
/// `a.txt.partial-<uuid>` → `Some(uuid)`。严格要求尾部是**合法 UUID**
/// （36 字符、含连字符），否则一律返回 `None`——`report.partial-backup`
/// 这种用户自己命名的文件不会被误判成临时文件。
pub fn partial_task_id(file_name: &str) -> Option<Uuid> {
    let (stem, tail) = file_name.rsplit_once(PARTIAL_SUFFIX)?;
    if stem.is_empty() || tail.is_empty() {
        return None;
    }
    Uuid::parse_str(tail).ok()
}

/// 判断一个文件是否是「目标 `stem` 的、可回收的 staging 临时文件」。
///
/// 条件（缺一不可）：
/// 1. 名字形如 `<stem>.partial-<合法 uuid>`——**目标名必须与本次传输的目标
///    一致**，否则用户自己命名的 `backup.partial-<uuid>` 会被误删；
/// 2. 其 task id **不在** `live_ids`（在途任务集合）里；
/// 3. `modified` 比 `now - min_age` 更早（mtime 在未来视为不可回收 ——
///    时钟回拨或远端服务器时间不准时，宁可不删）。
pub fn is_reclaimable_partial(
    file_name: &str,
    stem: &str,
    live_ids: &HashSet<Uuid>,
    modified: SystemTime,
    now: SystemTime,
    min_age: Duration,
) -> bool {
    let Some(rest) = file_name.strip_prefix(stem) else {
        return false;
    };
    let Some(uuid_part) = rest.strip_prefix(PARTIAL_SUFFIX) else {
        return false;
    };
    let Ok(id) = Uuid::parse_str(uuid_part) else {
        return false;
    };
    if live_ids.contains(&id) {
        return false;
    }
    now.duration_since(modified)
        .map(|age| age >= min_age)
        .unwrap_or(false)
}

/// 取路径的父目录。
///
/// `/a.txt` → `/`，`/home/u/a.txt` → `/home/u`，`a.txt` → `None`（没有父目录）。
pub fn parent_dir(path: &str) -> Option<String> {
    match path.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(slash) => Some(path[..slash].to_string()),
        None => None,
    }
}

/// 取路径的文件名部分（临时文件名的前缀）。
///
/// `/home/u/a.txt` → `a.txt`，`a.txt` → `a.txt`，`/` → `None`。
pub fn file_stem(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(slash) => {
            let name = &trimmed[slash + 1..];
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        }
        None => Some(trimmed),
    }
}

/// 把目录与文件名拼成远端路径（避免根目录下拼出 `//name`）。
pub fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// 回收本地目录里超龄的孤儿 staging 临时文件。
///
/// 只删除同时满足下列条件的文件：
/// - 名字形如 `<stem>.partial-<合法 uuid>`（`stem` 是本次传输的目标文件名）；
/// - 是**普通文件**——目录与符号链接一律不碰（用 `symlink_metadata` 判断，
///   避免顺着链接删到别处）；
/// - mtime 早于 `now - min_age`；
/// - 其 task id 不在 `live_ids` 里。
///
/// 返回被成功删除的路径列表。目录不存在、不可读时返回空列表——回收失败
/// 不应该影响传输任务本身。
pub async fn reclaim_local_partials(
    dir: &Path,
    stem: &str,
    live_ids: &HashSet<Uuid>,
    min_age: Duration,
    now: SystemTime,
) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(_) => return removed,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // 名字必须命中「<stem>.partial-」前缀 —— 其余文件连 mtime 都不看
        if !name.starts_with(stem) || !name.contains(PARTIAL_SUFFIX) {
            continue;
        }
        let path = entry.path();
        // symlink_metadata：不跟随链接，符号链接的 is_file() 为 false。
        let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if !is_reclaimable_partial(name, stem, live_ids, modified, now, min_age) {
            continue;
        }
        if tokio::fs::remove_file(&path).await.is_ok() {
            removed.push(path);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid_from(byte: u8) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[15] = byte;
        Uuid::from_bytes(bytes)
    }

    #[test]
    fn parses_task_id_from_a_matching_name() {
        let id = uuid_from(7);
        let name = format!("a.txt.partial-{id}");
        assert_eq!(partial_task_id(&name), Some(id));
    }

    #[test]
    fn rejects_names_without_a_valid_uuid_tail() {
        assert_eq!(partial_task_id("a.txt.partial-backup"), None);
        assert_eq!(partial_task_id("a.txt"), None);
        assert_eq!(partial_task_id(".partial-"), None);
        // 前缀为空 —— 那不是「给某个目标加的后缀」，是凭空捏的文件名
        assert_eq!(
            partial_task_id(".partial-2f1c9d3a-1111-2222-3333-444455556666"),
            None
        );
    }

    #[test]
    fn reclaim_requires_old_enough_and_not_live() {
        let id = uuid_from(3);
        let name = format!("a.txt.partial-{id}");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(7200);
        let fresh = now - Duration::from_secs(10);

        let empty = HashSet::new();
        assert!(is_reclaimable_partial(
            &name,
            "a.txt",
            &empty,
            old,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
        // 太新
        assert!(!is_reclaimable_partial(
            &name,
            "a.txt",
            &empty,
            fresh,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
        // 在途任务
        let live = HashSet::from([id]);
        assert!(!is_reclaimable_partial(
            &name,
            "a.txt",
            &live,
            old,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
        // 名字不匹配
        assert!(!is_reclaimable_partial(
            "a.txt",
            "a.txt",
            &empty,
            old,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
    }

    #[test]
    fn reclaim_only_touches_temps_of_the_same_target() {
        let id = uuid_from(5);
        let name = format!("a.txt.partial-{id}");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(7200);
        let empty = HashSet::new();

        // 别的目标的临时文件 —— 名字前缀对不上，不碰
        assert!(!is_reclaimable_partial(
            &name,
            "b.txt",
            &empty,
            old,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
        // 同前缀但不是 .partial- 形态
        assert!(!is_reclaimable_partial(
            "a.txt.bak",
            "a.txt",
            &empty,
            old,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
        // 前缀是目标名的子串也不行（`a.txt` 不是 `xa.txt.partial-<uuid>` 的前缀）
        assert!(!is_reclaimable_partial(
            &format!("xa.txt.partial-{id}"),
            "a.txt",
            &empty,
            old,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
    }

    #[test]
    fn future_mtime_is_never_reclaimed() {
        let id = uuid_from(4);
        let name = format!("a.txt.partial-{id}");
        let now = SystemTime::now();
        let future = now + Duration::from_secs(3600);
        assert!(!is_reclaimable_partial(
            &name,
            "a.txt",
            &HashSet::new(),
            future,
            now,
            PARTIAL_RECLAIM_MIN_AGE
        ));
    }

    #[test]
    fn parent_dir_handles_root_and_bare_names() {
        assert_eq!(parent_dir("/a.txt").as_deref(), Some("/"));
        assert_eq!(parent_dir("/home/u/a.txt").as_deref(), Some("/home/u"));
        assert_eq!(parent_dir("a.txt"), None);
    }

    #[test]
    fn file_stem_takes_the_last_path_segment() {
        assert_eq!(file_stem("/home/u/a.txt"), Some("a.txt"));
        assert_eq!(file_stem("a.txt"), Some("a.txt"));
        assert_eq!(file_stem("/home/u/"), Some("u"));
        assert_eq!(file_stem("/"), None);
    }

    #[test]
    fn join_remote_does_not_double_the_root_slash() {
        assert_eq!(join_remote("/", "a"), "/a");
        assert_eq!(join_remote("/home/u", "a"), "/home/u/a");
        assert_eq!(join_remote("/home/u/", "a"), "/home/u/a");
    }

    #[tokio::test]
    async fn reclaims_only_orphan_partial_files_of_the_same_target() {
        // 用 tempfile 拿到**独占**的临时目录：固定名字的目录会与并行运行的
        // 其它用例、以及系统临时目录的清理机制抢同一个路径。
        let temp = tempfile::tempdir().expect("temp dir");
        let dir = temp.path().to_path_buf();

        let orphan_id = uuid_from(1);
        let live_id = uuid_from(2);
        let orphan = dir.join(format!("a.txt.partial-{orphan_id}"));
        let live = dir.join(format!("a.txt.partial-{live_id}"));
        let other_target = dir.join(format!("b.txt.partial-{}", uuid_from(6)));
        let not_a_temp = dir.join("d.txt");
        let dir_named_like_temp = dir.join(format!("sub.partial-{}", uuid_from(4)));

        for path in [&orphan, &live, &other_target, &not_a_temp] {
            tokio::fs::write(path, b"x").await.unwrap();
        }
        tokio::fs::create_dir_all(&dir_named_like_temp)
            .await
            .unwrap();

        let live_ids = HashSet::from([live_id]);
        // min_age = 0 → 只考察「名字匹配 + 同一目标 + 非在途 + 是普通文件」
        let removed = reclaim_local_partials(
            &dir,
            "a.txt",
            &live_ids,
            Duration::ZERO,
            SystemTime::now() + Duration::from_secs(60),
        )
        .await;

        assert_eq!(removed, vec![orphan.clone()]);
        assert!(!orphan.exists());
        assert!(live.exists(), "在途任务的临时文件不能被回收");
        assert!(
            other_target.exists(),
            "其它目标的临时文件不在本次回收范围内"
        );
        assert!(not_a_temp.exists());
        assert!(dir_named_like_temp.exists(), "目录不能被当成临时文件删掉");
    }

    #[tokio::test]
    async fn reclaim_ignores_missing_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let missing = temp.path().join("does-not-exist");
        let removed = reclaim_local_partials(
            &missing,
            "a.txt",
            &HashSet::new(),
            PARTIAL_RECLAIM_MIN_AGE,
            SystemTime::now(),
        )
        .await;
        assert!(removed.is_empty());
    }
}
