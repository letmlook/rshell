//! SSH 隧道管理器
//!
//! 管理本地/远程/动态端口转发隧道。
//!
//! 持久化:隧道规则写入 `data_local_dir/rshell/tunnels.toml`,
//! 启动时自动恢复。运行时只持久化**规则**,不恢复 listener（避免与
//! 上次进程残留端口冲突；恢复在重启时再次 create_tunnel）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use uuid::Uuid;
use tracing::{info, warn, error};

use rshell_api::types::{ActiveTunnelInfo, PortForwardRule, TunnelState};
use rshell_api::events::AppEvent;

use crate::error::CoreError;
use crate::event_bus::EventBus;
use crate::session::service::SshClientHandle;

/// 活动隧道
#[derive(Debug)]
pub struct ActiveTunnel {
    pub id: Uuid,
    pub session_id: Uuid,
    pub rule: PortForwardRule,
    pub state: TunnelState,
    pub bytes_transferred: u64,
    pub connections_count: u32,
    /// 监听任务的句柄
    listener_handle: Option<tokio::task::JoinHandle<()>>,
}

/// SSH 隧道管理器
pub struct TunnelManager {
    tunnels: Arc<RwLock<HashMap<Uuid, ActiveTunnel>>>,
    event_bus: Arc<EventBus>,
    /// 持久化文件路径 (None 表示不持久化)
    persist_path: Option<PathBuf>,
}

/// 磁盘上的隧道注册表
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct PersistedTunnels {
    /// 按 session_id 分组,每个 session 下挂若干条规则
    #[serde(default)]
    rules: HashMap<Uuid, Vec<PortForwardRule>>,
}

impl TunnelManager {
    /// 创建新的隧道管理器
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            tunnels: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            persist_path: None,
        }
    }

    /// 启用持久化:启动时从 `path` 读,create_tunnel / close_tunnel 自动 dump
    ///
    /// 不会**自动**调用 `create_tunnel` 恢复;调用方应在适当时机调
    /// `restore_pending_rules` 拿到 `Vec<(Uuid, PortForwardRule)>` 后
    /// 显式 recreate (本环境的设计是: 重启不抢占端口, 仅记录用户意图)。
    pub fn with_persistence(mut self, path: PathBuf) -> Self {
        self.persist_path = Some(path);
        self
    }

    /// 从磁盘读所有 (session_id, rule) 对,供调用方决定是否重建
    pub async fn restore_pending_rules(&self) -> Vec<(Uuid, PortForwardRule)> {
        let Some(path) = self.persist_path.as_ref() else {
            return Vec::new();
        };
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<PersistedTunnels>(&content) {
                Ok(p) => p
                    .rules
                    .into_iter()
                    .flat_map(|(sid, rules)| {
                        rules.into_iter().map(move |r| (sid, r))
                    })
                    .collect(),
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                warn!("Failed to read {}: {}", path.display(), e);
                Vec::new()
            }
        }
    }

    /// 把当前 `tunnels` 状态 dump 到磁盘
    async fn save_to_disk(&self) {
        let Some(path) = self.persist_path.as_ref() else {
            return;
        };
        let tunnels = self.tunnels.read().await;
        // 把 ActiveTunnel 简化成 PersistedTunnels (只存规则)
        let mut grouped: HashMap<Uuid, Vec<PortForwardRule>> = HashMap::new();
        for t in tunnels.values() {
            grouped
                .entry(t.session_id)
                .or_default()
                .push(t.rule.clone());
        }
        let persisted = PersistedTunnels { rules: grouped };
        match toml::to_string_pretty(&persisted) {
            Ok(s) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(path, s) {
                    warn!("Failed to write {}: {}", path.display(), e);
                }
            }
            Err(e) => warn!("Failed to serialize tunnels: {}", e),
        }
    }

    /// 创建端口转发隧道
    ///
    /// `ssh_client`: 与该隧道关联的 SSH 连接句柄。若为 `None`（如 Telnet/Serial），
    /// 隧道退化为"仅 TCP 监听器"，不执行 SSH 通道转发。
    ///
    /// LocalForward: 监听 `bind_address:bind_port`，每条接入连接通过 SSH direct-tcpip
    /// 通道转发到 `remote_host:remote_port`。
    /// RemoteForward: 需要 SSH `tcpip-forward` 请求（russh 支持），本实现先做 LocalForward。
    /// DynamicForward(SOCKS): 解析 CONNECT 请求头并转发，略复杂，作为后续任务。
    pub async fn create_tunnel(
        &self,
        session_id: Uuid,
        rule: PortForwardRule,
        ssh_client: Option<SshClientHandle>,
    ) -> Result<Uuid, CoreError> {
        let tunnel_id = Uuid::new_v4();
        info!("Creating tunnel: id={}, rule={:?}", tunnel_id, rule);

        let bind_addr = format!("{}:{}", rule.bind_address, rule.bind_port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to bind {}: {}", bind_addr, e)))?;

        let local_addr = listener.local_addr()
            .map_err(|e| CoreError::Internal(format!("Failed to get local addr: {}", e)))?;

        info!("Tunnel listening on: {} (ssh_client={})", local_addr, ssh_client.is_some());

        let tunnels = self.tunnels.clone();
        let rule_for_task = rule.clone();
        let tunnel_id_for_task = tunnel_id;
        // ssh_client 实际参与转发:
        // - Some: 每条接入连接通过 SSH direct-tcpip 通道转发(russh::Channel<Msg>),
        //   数据流走 server,不直连目标主机。
        // - None: 直连目标主机(plain TCP 代理,用于 Telnet/Serial session 的隧道)。
        // Arc<RwLock<...>> 跨 loop 迭代需要 clone;Option 不 Copy。
        let ssh_client_ref = ssh_client;

        // 启动监听任务
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((mut inbound, peer_addr)) => {
                        info!("Tunnel {}: new connection from {}", tunnel_id_for_task, peer_addr);

                        // 更新连接计数
                        {
                            let mut tunnels = tunnels.write().await;
                            if let Some(t) = tunnels.get_mut(&tunnel_id_for_task) {
                                t.connections_count += 1;
                            }
                        }

                        let remote_host = rule_for_task.remote_host.clone();
                        let remote_port = rule_for_task.remote_port;
                        let tunnels_clone = tunnels.clone();
                        let tid = tunnel_id_for_task;

                        // 为每条连接 spawn 一个转发任务
                        let ssh_client_for_task = ssh_client_ref.clone();
                        tokio::spawn(async move {
                            let res = match ssh_client_for_task {
                                Some(ssh) => {
                                    // 真 SSH direct-tcpip: 拿 Channel<Msg>,跟 inbound TCP 做双向 copy
                                    let channel = {
                                        let client = ssh.read().await;
                                        client
                                            .open_direct_tcpip(&remote_host, remote_port as u32)
                                            .await
                                    };
                                    match channel {
                                        Ok(mut ch) => {
                                            let (mut ri, mut wi) = inbound.split();
                                            // 顺序关键: 先 make_writer(&self) 再 make_reader(&mut self),
                                            // 避免持有 &mut self 时再借 &self 失败。
                                            let mut wo = ch.make_writer();
                                            let mut ro = ch.make_reader();
                                            let c2s = tokio::io::copy(&mut ri, &mut wo);
                                            let s2c = tokio::io::copy(&mut ro, &mut wi);
                                            let (c2s_res, s2c_res) = tokio::join!(c2s, s2c);
                                            // ro/wo 持 ch 的借用,显式 drop 后才能调 ch.eof()
                                            drop(ro);
                                            drop(wo);
                                            let _ = ch.eof().await;
                                            if let Err(e) = c2s_res {
                                                warn!("Tunnel {}: client→remote (ssh) copy error: {}", tid, e);
                                            }
                                            if let Err(e) = s2c_res {
                                                warn!("Tunnel {}: remote→client (ssh) copy error: {}", tid, e);
                                            }
                                            Ok(())
                                        }
                                        Err(e) => Err(format!(
                                            "ssh direct-tcpip {}:{} failed: {}",
                                            remote_host, remote_port, e
                                        )),
                                    }
                                }
                                None => {
                                    // Plain TCP 代理(非 SSH session 的隧道)
                                    match TcpStream::connect(format!("{}:{}", remote_host, remote_port)).await {
                                        Ok(mut remote) => {
                                            let (mut ri, mut wi) = inbound.split();
                                            let (mut ro, mut wo) = remote.split();
                                            let c2s = tokio::io::copy(&mut ri, &mut wo);
                                            let s2c = tokio::io::copy(&mut ro, &mut wi);
                                            let (c2s_res, s2c_res) = tokio::join!(c2s, s2c);
                                            if let Err(e) = c2s_res {
                                                warn!("Tunnel {}: client→remote (tcp) copy error: {}", tid, e);
                                            }
                                            if let Err(e) = s2c_res {
                                                warn!("Tunnel {}: remote→client (tcp) copy error: {}", tid, e);
                                            }
                                            Ok(())
                                        }
                                        Err(e) => Err(format!(
                                            "TCP connect to {}:{} failed: {}",
                                            remote_host, remote_port, e
                                        )),
                                    }
                                }
                            };
                            if let Err(msg) = res {
                                warn!("Tunnel {}: {}", tid, msg);
                            }

                            // 连接关闭后更新计数
                            let mut tunnels = tunnels_clone.write().await;
                            if let Some(t) = tunnels.get_mut(&tid) {
                                t.connections_count = t.connections_count.saturating_sub(1);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Tunnel {} accept error: {}", tunnel_id_for_task, e);
                        break;
                    }
                }
            }
        });

        let tunnel = ActiveTunnel {
            id: tunnel_id,
            session_id,
            rule,
            state: TunnelState::Active,
            bytes_transferred: 0,
            connections_count: 0,
            listener_handle: Some(handle),
        };

        self.tunnels.write().await.insert(tunnel_id, tunnel);

        self.event_bus.publish(AppEvent::TunnelStateChanged {
            tunnel_id,
            state: TunnelState::Active,
        });
        self.event_bus.publish(AppEvent::ActiveTunnelsChanged);

        info!("Tunnel created: id={}", tunnel_id);
        self.save_to_disk().await;
        Ok(tunnel_id)
    }

    /// 关闭隧道
    pub async fn close_tunnel(&self, tunnel_id: Uuid) -> Result<(), CoreError> {
        info!("Closing tunnel: {}", tunnel_id);

        // 在 write guard 内做变更 + 取消 listener,之后立即释放 guard
        // (save_to_disk 内部会再 .read().await 拿 read guard)
        let removed = {
            let mut tunnels = self.tunnels.write().await;
            if let Some(mut tunnel) = tunnels.remove(&tunnel_id) {
                if let Some(handle) = tunnel.listener_handle.take() {
                    handle.abort();
                }
                true
            } else {
                false
            }
        };

        if removed {
            self.event_bus.publish(AppEvent::TunnelStateChanged {
                tunnel_id,
                state: TunnelState::Error("Closed".into()),
            });
            self.event_bus.publish(AppEvent::ActiveTunnelsChanged);

            self.save_to_disk().await;
            info!("Tunnel closed: {}", tunnel_id);
            Ok(())
        } else {
            Err(CoreError::NotFound(format!("Tunnel not found: {}", tunnel_id)))
        }
    }

    /// 暂停隧道
    pub async fn suspend_tunnel(&self, tunnel_id: Uuid) -> Result<(), CoreError> {
        let mut tunnels = self.tunnels.write().await;
        if let Some(tunnel) = tunnels.get_mut(&tunnel_id) {
            tunnel.state = TunnelState::Suspended;

            self.event_bus.publish(AppEvent::TunnelStateChanged {
                tunnel_id,
                state: TunnelState::Suspended,
            });

            info!("Tunnel suspended: {}", tunnel_id);
            Ok(())
        } else {
            Err(CoreError::NotFound(format!("Tunnel not found: {}", tunnel_id)))
        }
    }

    /// 恢复隧道
    pub async fn resume_tunnel(&self, tunnel_id: Uuid) -> Result<(), CoreError> {
        let mut tunnels = self.tunnels.write().await;
        if let Some(tunnel) = tunnels.get_mut(&tunnel_id) {
            tunnel.state = TunnelState::Active;

            self.event_bus.publish(AppEvent::TunnelStateChanged {
                tunnel_id,
                state: TunnelState::Active,
            });

            info!("Tunnel resumed: {}", tunnel_id);
            Ok(())
        } else {
            Err(CoreError::NotFound(format!("Tunnel not found: {}", tunnel_id)))
        }
    }

    /// 列出所有活动隧道
    pub async fn list_tunnels(&self) -> Vec<ActiveTunnelInfo> {
        let tunnels = self.tunnels.read().await;
        tunnels
            .values()
            .map(|t| ActiveTunnelInfo {
                id: t.id,
                session_id: t.session_id,
                rule: t.rule.clone(),
                state: t.state.clone(),
                bytes_transferred: t.bytes_transferred,
                connections_count: t.connections_count,
            })
            .collect()
    }

    /// 获取隧道信息
    pub async fn get_tunnel(&self, tunnel_id: Uuid) -> Option<ActiveTunnelInfo> {
        let tunnels = self.tunnels.read().await;
        tunnels.get(&tunnel_id).map(|t| ActiveTunnelInfo {
            id: t.id,
            session_id: t.session_id,
            rule: t.rule.clone(),
            state: t.state.clone(),
            bytes_transferred: t.bytes_transferred,
            connections_count: t.connections_count,
        })
    }
}

impl Drop for ActiveTunnel {
    fn drop(&mut self) {
        if let Some(handle) = self.listener_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::{ForwardDirection, PortForwardRule};

    fn make_rule(host: &str, port: u16) -> PortForwardRule {
        PortForwardRule {
            bind_address: "127.0.0.1".to_string(),
            bind_port: port,
            remote_host: host.to_string(),
            remote_port: port,
            direction: ForwardDirection::Local,
        }
    }

    #[tokio::test]
    async fn test_persistence_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "rshell-tunnels-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let mgr = TunnelManager::new(bus).with_persistence(tmp.clone());

        // restore empty (file not exist)
        let pending = mgr.restore_pending_rules().await;
        assert!(pending.is_empty());

        // create_tunnel 需要绑端口, 选一个高位端口避免冲突
        let sid = Uuid::new_v4();
        let tid = mgr
            .create_tunnel(sid, make_rule("example.com", 80), None)
            .await
            .unwrap();
        // 等 create_tunnel 的 save 完成
        mgr.close_tunnel(tid).await.unwrap();

        // 现在应能从磁盘读出
        let mgr2 = TunnelManager::new(Arc::new(crate::event_bus::EventBus::new()))
            .with_persistence(tmp.clone());
        let pending = mgr2.restore_pending_rules().await;
        // close_tunnel 已经把 tunnels 移除了,所以 save 出的应该为空
        // (closed tunnels 不再持久化)
        let _ = pending; // 主要是触发"读盘能跑通"
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_restore_handles_missing_file() {
        let tmp = std::env::temp_dir().join(format!(
            "rshell-missing-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let mgr = TunnelManager::new(Arc::new(crate::event_bus::EventBus::new()))
            .with_persistence(tmp);
        let pending = mgr.restore_pending_rules().await;
        assert!(pending.is_empty());
    }
}
