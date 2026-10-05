//! 主机密钥管理器
//!
//! 管理 known_hosts 文件，验证服务器身份。
//!
//! 文件格式遵循 OpenSSH 标准 known_hosts 规范：
//! ```text
//! <host_pattern> <keytype> <base64-key> [comment]
//! ```
//! 这样可以与系统 `ssh-keygen` 等工具互操作。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use rshell_api::types::{HostKeyEntry, TrustLevel};
use tracing::{debug, info, warn};

use crate::error::CoreError;

/// 单次调用"先落盘、后改内存"所需的行级变更。
///
/// 信任/删除都要求持久化成功才算生效（CLAUDE.md：永久信任保存成功后才接受握手），
/// 所以这次要写进文件的内容必须能表达成"某 host 模式的行替换成 X / 整行删除"，
/// 而不是先改内存 map 再从 map 反推文件内容。
enum Overlay<'a> {
    /// 该 host 模式的行替换为 `line`（`line` 自带换行）
    Upsert { map_key: &'a str, line: &'a str },
    /// 该 host 模式的行全部删除
    Remove { map_key: &'a str },
}

/// 一个 host 模式在文件里的归属裁决
enum Owned {
    /// 归本应用管，且就渲染成这一行
    Line(String),
    /// 归本应用管，但这一行不该再出现在文件里
    /// （条目已被删除 / 非 Trusted / 遗留 SHA256 指纹无法用于 OpenSSH 比对）
    Dropped,
}

/// 主机密钥管理器
pub struct HostKeyManager {
    /// host:port -> HostKeyEntry
    entries: Arc<RwLock<HashMap<String, HostKeyEntry>>>,
    known_hosts_path: PathBuf,
}

impl HostKeyManager {
    /// 创建新的主机密钥管理器
    pub fn new(known_hosts_path: PathBuf) -> Self {
        let manager = Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            known_hosts_path,
        };

        // 加载 known_hosts 文件
        let _ = manager.load_known_hosts();

        manager
    }

    /// 加载 known_hosts 文件（OpenSSH 标准格式）
    fn load_known_hosts(&self) -> Result<(), CoreError> {
        if !self.known_hosts_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.known_hosts_path)
            .map_err(|e| CoreError::Internal(format!("Failed to read known_hosts: {}", e)))?;

        let mut entries = HashMap::new();
        let now = Utc::now().to_rfc3339();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // OpenSSH known_hosts 行格式：
            //   <host_pattern>[,<host_pattern>...] <keytype> <base64-key> [comment]
            // hashed 条目：|1|base64(salt)|base64(hash) <keytype> <base64-key> ...
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }

            let host_field = parts[0];
            // 跳过 hashed 条目（无法精确匹配 host 模式）
            if host_field.starts_with("|1|") {
                continue;
            }
            // OpenSSH 标记行（`@revoked` / `@cert-authority` / `@known-hosts-command`）
            // 格式是 `<marker> <host-pattern> <keytype> <key>`：按普通行解析会把
            // 标记名当成 host、keytype 当成 fingerprint，重写时还会整段丢掉密钥
            // 材料（R3-12）。这里不建模——文件内容由 rewrite 逐行原样保留。
            if host_field.starts_with('@') {
                debug!(line = %line, "Skipping OpenSSH marker line in known_hosts");
                continue;
            }

            let key_type = parts[1].to_string();
            // key_blob 直接作为 fingerprint 存储（用于快速等价比较与显示）。
            // 真实的安全校验依赖基于 base64 内容的精确匹配，而非 SHA256 哈希。
            let fingerprint = parts[2].to_string();

            // 对 host_field 中的每个 host 模式，提取 host/port 并加入 entries
            for pattern in host_field.split(',') {
                let (host, port) = parse_host_port(pattern, 22);
                entries.insert(
                    map_key_of(pattern),
                    HostKeyEntry {
                        host,
                        port,
                        key_type: key_type.clone(),
                        fingerprint: fingerprint.clone(),
                        trust_level: TrustLevel::Trusted,
                        first_seen: now.clone(),
                        last_seen: now.clone(),
                    },
                );
            }
        }

        *self.entries.write().expect("host key lock poisoned") = entries;

        info!(
            "Loaded {} entries from known_hosts",
            self.entries.read().expect("host key lock poisoned").len()
        );
        Ok(())
    }

    /// 渲染完整的 known_hosts 内容（OpenSSH 标准格式，人可读）。
    ///
    /// 以**磁盘上现有文件**为基底逐行裁决，而不是从内存 map 整体重写。
    /// 旧实现重建整份文件，解析器不建模的行——注释、OpenSSH 标记行（`@revoked`
    /// 等）、`|1|…` 哈希条目、带尾注释的第三方行——会被静默删除（只留一条
    /// `warn!`），用户在 ssh-keyscan / ssh-keygen 之后写进文件的内容就此消失。
    ///
    /// 裁决规则：
    /// - 非本应用形状的行（见 `parse_app_owned_line`）→ **逐字节原样保留**；
    /// - 本应用形状的行 → 按 `owned` 里的规范行替换；已被删除/非 Trusted 的
    ///   host 模式 → 丢弃该模式；同一行里不归本应用管的其它 host 模式 → 用本行
    ///   自带的算法名与密钥材料拆成独立行保留，绝不因为"重写这一行"而丢别人的 key；
    /// - 文件里还没有的条目按 host 排序追加在末尾（输出稳定、diff 友好）。
    fn render_known_hosts(
        &self,
        entries: &HashMap<String, HostKeyEntry>,
        overlay: &[Overlay<'_>],
    ) -> Result<String, CoreError> {
        // 1) 本应用当前"拥有"的 host 模式 → 规范行
        let mut owned: HashMap<String, Owned> = HashMap::new();
        for (map_key, entry) in entries {
            if entry.trust_level != TrustLevel::Trusted {
                owned.insert(map_key.clone(), Owned::Dropped);
                continue;
            }
            // 如果 fingerprint 字段实际是 openssh 格式（ssh-xxx base64...）则原样写回；
            // 否则（旧的 SHA256:xxx 字符串）跳过——已不兼容新格式
            if entry.fingerprint.starts_with("SHA256:") {
                warn!(
                    host = %entry.host,
                    port = entry.port,
                    "Skipping legacy SHA256-fingerprint entry during rewrite"
                );
                owned.insert(map_key.clone(), Owned::Dropped);
                continue;
            }
            owned.insert(map_key.clone(), Owned::Line(render_entry_line(entry)));
        }
        // 2) 本次调用的行级变更（尚未反映到 entries 里）
        for ov in overlay {
            match ov {
                Overlay::Upsert { map_key, line } => {
                    owned.insert((*map_key).to_string(), Owned::Line((*line).to_string()));
                }
                Overlay::Remove { map_key } => {
                    owned.insert((*map_key).to_string(), Owned::Dropped);
                }
            }
        }

        let existing = match std::fs::read_to_string(&self.known_hosts_path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            // 读不出来就不能重写：宁可不写，也不用一份空内容盖掉用户的信任库
            Err(e) => {
                return Err(CoreError::Internal(format!(
                    "Failed to read known_hosts: {}",
                    e
                )))
            }
        };

        let mut out = String::with_capacity(existing.len() + 256);
        let mut emitted: HashSet<String> = HashSet::new();
        for line in existing.lines() {
            let Some(parsed) = parse_app_owned_line(line) else {
                out.push_str(line);
                out.push('\n');
                continue;
            };
            let mut foreign_patterns: Vec<&str> = Vec::new();
            for pattern in &parsed.patterns {
                let map_key = map_key_of(pattern);
                match owned.get(map_key.as_str()) {
                    Some(Owned::Line(rendered)) => {
                        if emitted.insert(map_key) {
                            out.push_str(rendered);
                            out.push('\n');
                        }
                    }
                    Some(Owned::Dropped) => {}
                    None => foreign_patterns.push(pattern),
                }
            }
            if !foreign_patterns.is_empty() {
                out.push_str(&format!(
                    "{} {} {}\n",
                    foreign_patterns.join(","),
                    parsed.key_type,
                    parsed.blob
                ));
            }
        }

        // 3) 追加文件里还没有的条目（按 host 排序，输出稳定）
        let mut missing: Vec<&String> = owned
            .iter()
            .filter(|(map_key, verdict)| {
                matches!(verdict, Owned::Line(_)) && !emitted.contains(map_key.as_str())
            })
            .map(|(map_key, _)| map_key)
            .collect();
        missing.sort();
        for map_key in missing {
            if let Some(Owned::Line(rendered)) = owned.get(map_key) {
                out.push_str(rendered);
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// 渲染 + **原子**落盘：先写同目录暂存文件再 rename。
    ///
    /// 旧实现 `fs::write` 原地截断，崩溃或磁盘写满会留下空/半份文件，
    /// 已信任的主机静默退回"未知"并重新弹窗（R3-12）。`fs::rename` 在同一
    /// 文件系统内是原子的：Windows 走 `MoveFileEx(MOVEFILE_REPLACE_EXISTING)`
    /// 可覆盖旧文件，POSIX 走 `rename(2)`，读者要么看到旧文件要么看到新文件。
    fn persist_known_hosts(
        &self,
        entries: &HashMap<String, HostKeyEntry>,
        overlay: &[Overlay<'_>],
    ) -> Result<(), CoreError> {
        let content = self.render_known_hosts(entries, overlay)?;

        if let Some(parent) = self.known_hosts_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 暂存名固定为 `<known_hosts>.tmp`：不与真实文件同 inode，rename 才能原子替换
        let mut tmp = self.known_hosts_path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if let Err(e) = std::fs::write(&tmp, content.as_bytes()) {
            return Err(CoreError::Internal(format!(
                "Failed to write known_hosts: {}",
                e
            )));
        }
        if let Err(e) = std::fs::rename(&tmp, &self.known_hosts_path) {
            // 替换失败时清掉暂存文件，别把半份内容留在信任库目录里
            let _ = std::fs::remove_file(&tmp);
            return Err(CoreError::Internal(format!(
                "Failed to replace known_hosts: {}",
                e
            )));
        }
        Ok(())
    }

    // 已知条目但指纹不一致（可能中间人）由协议层 rshell-protocol::ssh::client
    // 的 verify_known_hosts → check_server_key 决策链处理，不再在此重复判定。

    /// 信任主机密钥
    ///
    /// `key_type` 应是 OpenSSH 算法名（如 `ssh-ed25519`、`rsa-sha2-256`）。
    /// `key_blob` 应是 base64 编码的公钥（无 keytype 前缀）。
    pub async fn trust_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        key_blob: &str,
    ) -> Result<(), CoreError> {
        let key = format!("{}:{}", host, port);
        let now = Utc::now().to_rfc3339();

        let entry = HostKeyEntry {
            host: host.to_string(),
            port,
            key_type: key_type.to_string(),
            fingerprint: key_blob.to_string(),
            trust_level: TrustLevel::Trusted,
            first_seen: now.clone(),
            last_seen: now,
        };

        // 先落盘、后改内存（R3-13）。写失败时（只读卷 / 权限 / 磁盘满）内存 map
        // 里不会留下"只在内存里可信"的条目——否则 `list_hosts` 谎报已信任，而
        // 下一次无关的 save 又会把它写进 known_hosts，全程没有新的用户决策。
        // 握手仍然安全：调用方在写失败时把决定解析为 accept=false。
        // 写锁覆盖"读内存 → 算内容 → 落盘 → 改内存"，并发 trust/delete 不会互相覆盖。
        let line = render_entry_line(&entry);
        let mut entries = self.entries.write().expect("host key lock poisoned");
        self.persist_known_hosts(
            &entries,
            &[Overlay::Upsert {
                map_key: &key,
                line: &line,
            }],
        )?;
        entries.insert(key, entry);

        info!("Host key trusted: {}:{}", host, port);
        Ok(())
    }

    /// 删除主机密钥
    pub async fn delete_host_key(&self, host: &str, port: u16) -> Result<(), CoreError> {
        let key = format!("{}:{}", host, port);
        // 与 trust 一样先落盘：写失败时磁盘记录与内存条目都还在，二者不会给出
        // 相反的答案（内存已删、协议层重读文件仍信任）。
        let mut entries = self.entries.write().expect("host key lock poisoned");
        self.persist_known_hosts(&entries, &[Overlay::Remove { map_key: &key }])?;
        entries.remove(&key);
        info!("Host key deleted: {}:{}", host, port);
        Ok(())
    }

    /// 列出所有已知主机
    pub async fn list_hosts(&self) -> Vec<HostKeyEntry> {
        self.entries
            .read()
            .expect("host key lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 更新最后访问时间
    pub async fn update_last_seen(&self, host: &str, port: u16) {
        let key = format!("{}:{}", host, port);
        let mut entries = self.entries.write().expect("host key lock poisoned");
        if let Some(entry) = entries.get_mut(&key) {
            entry.last_seen = Utc::now().to_rfc3339();
        }
    }
}

/// host 模式 → 内存 map 的键（`host:port`）。
///
/// 加载与重写判定必须用**同一套**解析与拼键，否则文件行会被误判成"不归本应用管"
/// （多出一份重复行）或反过来吃掉别人的行。
fn map_key_of(pattern: &str) -> String {
    let (host, port) = parse_host_port(pattern, 22);
    format!("{}:{}", host, port)
}

/// 条目 → 本应用拥有的一行（OpenSSH 标准格式：端口 22 写裸 host，其余 `[host]:port`）
fn render_entry_line(entry: &HostKeyEntry) -> String {
    let host_pattern = if entry.port == 22 {
        entry.host.clone()
    } else {
        format!("[{}]:{}", entry.host, entry.port)
    };
    format!(
        "{} {} {}\n",
        host_pattern, entry.key_type, entry.fingerprint
    )
}

/// 形状与本应用写出**完全一致**的 known_hosts 行
struct OwnedLine<'a> {
    /// host 模式字段按 `,` 拆开的结果
    patterns: Vec<&'a str>,
    key_type: &'a str,
    /// base64 公钥（内容里不含空格，切分不会破坏它）
    blob: &'a str,
}

/// 只把"本应用形状"的行当作归本应用管。
///
/// 本应用只写 3 个字段（host 模式、算法名、base64 公钥，不带尾注释），因此判定为：
/// 恰好 3 个非空白字段，且首字段既不是 `@` 标记也不是 `|1|…` 哈希条目。
/// 其余一律视为外部行——注释、标记行、哈希行、带尾注释或字段不足的行——逐字节保留。
fn parse_app_owned_line(line: &str) -> Option<OwnedLine<'_>> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 || parts[0].starts_with('@') || parts[0].starts_with('|') {
        return None;
    }
    Some(OwnedLine {
        patterns: parts[0].split(',').collect(),
        key_type: parts[1],
        blob: parts[2],
    })
}

/// 解析 OpenSSH host pattern 为 (host, port)
/// 支持 `[host]:port` / `host:port` / `host` 及裸 IPv6 字面量（如 `::1`）
fn parse_host_port(pattern: &str, default_port: u16) -> (String, u16) {
    let pattern = pattern.trim_end_matches(',');
    if let Some(idx) = pattern.find("]:") {
        let host = &pattern[..idx + 1];
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = pattern[idx + 2..].parse::<u16>().unwrap_or(default_port);
        return (host.to_string(), port);
    }
    // 裸 IPv6 字面量：盲切 rfind(':') 会得到错误 host/port。OpenSSH 对
    // 端口 22 的 IPv6 主机即写裸地址，按 default_port 处理（R2-06）
    if pattern.parse::<std::net::IpAddr>().is_ok() {
        return (pattern.to_string(), default_port);
    }
    if let Some(idx) = pattern.rfind(':') {
        let host = &pattern[..idx];
        let port = pattern[idx + 1..].parse::<u16>().unwrap_or(default_port);
        return (host.to_string(), port);
    }
    (pattern.to_string(), default_port)
}

/// 从 OpenSSH base64 公钥字符串计算 SHA256 指纹（用于显示）
///
/// 当前实现直接返回原 blob — `key_blob` 本身就是用于等价比较的稳定标识。
/// 显示用的人类可读指纹可由 UI 层根据 base64 blob 自行计算后展示。
fn _fingerprint_helper(key_blob: &str) -> String {
    let _ = key_blob;
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn permanent_trust_persists_across_manager_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let manager = HostKeyManager::new(path.clone());
        manager
            .trust_host_key("example.test", 2222, "ssh-ed25519", "AAAAkey")
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("[example.test]:2222 ssh-ed25519 AAAAkey"));

        // 重启后应从磁盘恢复条目（握手期的比对由协议层 verify_known_hosts 负责）
        let restored = HostKeyManager::new(path);
        let entries = restored.entries.read().unwrap();
        let entry = entries
            .get("example.test:2222")
            .expect("重启后条目应被加载");
        assert_eq!(entry.fingerprint, "AAAAkey");
        assert_eq!(entry.trust_level, TrustLevel::Trusted);
    }

    /// R2-06 验收：端口 22 的 IPv6 主机写裸地址（OpenSSH 格式），重启加载
    /// 时不得被 rfind(':') 误切成 host=":" port=1
    #[tokio::test]
    async fn ipv6_host_persists_as_bare_address_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let manager = HostKeyManager::new(path.clone());
        manager
            .trust_host_key("::1", 22, "ssh-ed25519", "AAAAkey6")
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            on_disk.contains("::1 ssh-ed25519 AAAAkey6"),
            "端口 22 的 IPv6 主机必须写裸地址，实际：{}",
            on_disk
        );

        let restored = HostKeyManager::new(path);
        let entries = restored.entries.read().unwrap();
        let entry = entries
            .get("::1:22")
            .expect("裸 IPv6 条目应以 host=\"::1\" port=22 恢复");
        assert_eq!(entry.host, "::1");
        assert_eq!(entry.port, 22);
    }

    #[test]
    fn parse_host_port_handles_ipv6_literals() {
        // 裸 IPv6 按默认端口处理，不得切分冒号（回归：切出 host=":" port=1）
        assert_eq!(parse_host_port("::1", 22), ("::1".to_string(), 22));
        assert_eq!(parse_host_port("fe80::1", 22), ("fe80::1".to_string(), 22));
        // 方括号与 host:port 写法行为不变
        assert_eq!(parse_host_port("[::1]:2222", 22), ("::1".to_string(), 2222));
        assert_eq!(
            parse_host_port("example.test:2222", 22),
            ("example.test".to_string(), 2222)
        );
        assert_eq!(
            parse_host_port("example.test", 22),
            ("example.test".to_string(), 22)
        );
    }

    /// R3-12 回归：known_hosts 里的"外部行"必须逐字节保留。
    ///
    /// 旧实现从内存 map 整体重写文件，下一次 trust/delete 会静默删掉注释、
    /// OpenSSH 哈希条目和 `@revoked` 标记行（含其 base64 密钥材料）。
    /// 本测试同时覆盖"更新既有 app 条目"和"追加新条目"两条路径。
    #[tokio::test]
    async fn trust_preserves_foreign_known_hosts_lines_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let comment = "# managed by rshell — keys below this line are app-owned";
        let hashed = "|1|c2FsdA==|aGFzaA== ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHASHED";
        let revoked = "@revoked bad.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIREVBQ0VEC";
        let owned_old = "owned.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOLDKEY";
        std::fs::write(
            &path,
            format!("{comment}\n{hashed}\n{revoked}\n{owned_old}\n"),
        )
        .unwrap();

        let manager = HostKeyManager::new(path.clone());
        // 同一 host 换 key（更新既有 app 行）
        manager
            .trust_host_key(
                "owned.example",
                22,
                "ssh-ed25519",
                "AAAAC3NzaC1lZDI1NTE5AAAAINEWKEY",
            )
            .await
            .unwrap();
        // 新 host（追加）
        manager
            .trust_host_key("new.example", 2222, "ssh-ed25519", "AAAAnew")
            .await
            .unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        for foreign in [comment, hashed, revoked] {
            assert!(
                after.contains(foreign),
                "外部行必须逐字节保留：{}\n实际内容：\n{}",
                foreign,
                after
            );
        }
        assert!(
            after.contains("owned.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINEWKEY")
                && !after.contains(owned_old),
            "app 自己拥有的行应被就地更新，实际内容：\n{}",
            after
        );
        assert!(
            after.contains("[new.example]:2222 ssh-ed25519 AAAAnew"),
            "新条目应被追加，实际内容：\n{}",
            after
        );
        // 原子写不留暂存文件
        assert!(
            !std::path::Path::new(&format!("{}.tmp", path.display())).exists(),
            "rename 成功后不应残留暂存文件"
        );

        // `@revoked` 行不再被按错偏移解析成"host = @revoked"
        let hosts = manager.list_hosts().await;
        assert!(
            hosts
                .iter()
                .all(|e| e.host != "@revoked" && e.key_type != "bad.example"),
            "OpenSSH 标记行不得进入内存 map，实际：{:?}",
            hosts
        );
        assert!(hosts
            .iter()
            .any(|e| e.host == "owned.example" && e.fingerprint.ends_with("NEWKEY")));
        assert!(hosts
            .iter()
            .any(|e| e.host == "new.example" && e.port == 2222));
    }

    /// R3-12 回归：`delete_host_key` 同样不得吃掉外部行。
    #[tokio::test]
    async fn delete_preserves_foreign_known_hosts_lines_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let comment = "# keep me";
        let revoked = "@revoked bad.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIREVBQ0VEC";
        let hashed = "|1|c2FsdA==|aGFzaA== ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHASHED";
        std::fs::write(
            &path,
            format!("{comment}\n{revoked}\n{hashed}\nowned.example ssh-ed25519 AAAAowned\n"),
        )
        .unwrap();

        let manager = HostKeyManager::new(path.clone());
        manager.delete_host_key("owned.example", 22).await.unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            format!("{comment}\n{revoked}\n{hashed}\n"),
            "只应删掉 app 自己拥有的那一行"
        );
    }

    /// 造一个"落盘必然失败"的 store：原子写先把内容写到 `<known_hosts>.tmp`，
    /// 在那里预先建一个同名目录，暂存文件写入必然失败（Windows / POSIX 皆然）。
    /// 真实 known_hosts 仍可读，所以条目能被正常加载——delete 路径也覆盖得到。
    fn manager_with_failing_write(path: PathBuf) -> HostKeyManager {
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        std::fs::create_dir_all(PathBuf::from(tmp)).unwrap();
        HostKeyManager::new(path)
    }

    /// R3-13 回归：落盘失败时内存 map 不得留下"只在内存里可信"的条目。
    /// 旧实现先 insert 再 save，写失败后 `list_hosts` 谎报已信任，而下一次
    /// 无关的 save 会把它写进 known_hosts——全程没有新的用户决策。
    ///
    /// 失败注入用"父路径是普通文件"：`create_dir_all` 与最终写入都必然失败，
    /// 与落盘是原地截断还是 temp+rename 无关。
    #[tokio::test]
    async fn failed_trust_write_does_not_trust_host_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        let manager = HostKeyManager::new(blocker.join("known_hosts"));

        let result = manager
            .trust_host_key("example.test", 22, "ssh-ed25519", "AAAAkey")
            .await;
        assert!(result.is_err(), "落盘必然失败，trust_host_key 必须返回 Err");

        assert!(
            manager
                .entries
                .read()
                .expect("host key lock poisoned")
                .is_empty(),
            "写失败后内存 map 不得包含该条目"
        );
        assert!(
            manager.list_hosts().await.is_empty(),
            "list_hosts 不得把未落盘的信任报成已信任"
        );
    }

    /// R3-13 回归（镜像）：删除写失败时条目必须仍在内存、也仍在磁盘上。
    /// 失败注入靠原子写的暂存路径（`<known_hosts>.tmp` 已被同名目录占位），
    /// 这样真实 known_hosts 仍可读、条目能被加载——旧实现的原地截断写法在此
    /// 不会失败，所以这个用例也顺带钉住了"删除必须真的落盘才算数"。
    #[tokio::test]
    async fn failed_delete_write_keeps_host_trusted_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        {
            let seeded = HostKeyManager::new(path.clone());
            seeded
                .trust_host_key("example.test", 22, "ssh-ed25519", "AAAAkey")
                .await
                .unwrap();
        }

        // 落盘必然失败的 store：暂存文件位置已被目录占位，条目照常从磁盘加载
        let failing = manager_with_failing_write(path.clone());
        let result = failing.delete_host_key("example.test", 22).await;
        assert!(
            result.is_err(),
            "落盘必然失败，delete_host_key 必须返回 Err"
        );
        assert!(
            failing
                .entries
                .read()
                .expect("host key lock poisoned")
                .contains_key("example.test:22"),
            "删除写失败时内存条目必须保留（磁盘上仍有记录，二者不能分叉）"
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("example.test ssh-ed25519 AAAAkey"),
            "删除写失败时磁盘记录必须原样保留"
        );
    }
}
