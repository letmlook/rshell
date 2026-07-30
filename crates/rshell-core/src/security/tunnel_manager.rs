//! SSH 隧道管理器
//!
//! 管理本地/远程/动态端口转发隧道。

use std::collections::HashMap;
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
}

impl TunnelManager {
    /// 创建新的隧道管理器
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            tunnels: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
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
        Ok(tunnel_id)
    }

    /// 关闭隧道
    pub async fn close_tunnel(&self, tunnel_id: Uuid) -> Result<(), CoreError> {
        info!("Closing tunnel: {}", tunnel_id);

        let mut tunnels = self.tunnels.write().await;
        if let Some(mut tunnel) = tunnels.remove(&tunnel_id) {
            // 取消监听任务
            if let Some(handle) = tunnel.listener_handle.take() {
                handle.abort();
            }

            self.event_bus.publish(AppEvent::TunnelStateChanged {
                tunnel_id,
                state: TunnelState::Error("Closed".into()),
            });
            self.event_bus.publish(AppEvent::ActiveTunnelsChanged);

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
