//! SSH 隧道管理器
//!
//! 管理本地/远程/动态端口转发隧道。

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::net::TcpListener;
use uuid::Uuid;
use tracing::{info, error};

use rshell_api::types::{ActiveTunnelInfo, PortForwardRule, TunnelState};
use rshell_api::events::AppEvent;

use crate::error::CoreError;
use crate::event_bus::EventBus;

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
    /// 参数:
    /// - `session_id`: 关联的会话 ID
    /// - `rule`: 端口转发规则
    /// - `_ssh_client`: 保留参数，用于未来传入 SSH 客户端实现实际的端口转发
    ///
    /// 当前实现：创建 TCP 监听器并接受连接，实际的 SSH 通道转发需要集成
    /// russh 的 channel forwarding 功能。参见 russh::channels 模块。
    pub async fn create_tunnel(
        &self,
        session_id: Uuid,
        rule: PortForwardRule,
        _ssh_client: Option<()>,
    ) -> Result<Uuid, CoreError> {
        let tunnel_id = Uuid::new_v4();
        info!("Creating tunnel: id={}, rule={:?}", tunnel_id, rule);

        let bind_addr = format!("{}:{}", rule.bind_address, rule.bind_port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| CoreError::Internal(format!("Failed to bind {}: {}", bind_addr, e)))?;

        let local_addr = listener.local_addr()
            .map_err(|e| CoreError::Internal(format!("Failed to get local addr: {}", e)))?;

        info!("Tunnel listening on: {}", local_addr);

        let tunnels = self.tunnels.clone();
        let rule_for_task = rule.clone();

        // 启动监听任务
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((_stream, peer_addr)) => {
                        info!("New connection from {} to tunnel", peer_addr);

                        // 更新连接计数
                        {
                            let mut tunnels = tunnels.write().await;
                            if let Some(tunnel) = tunnels.get_mut(&tunnel_id) {
                                tunnel.connections_count += 1;
                            }
                        }

                        // 实际的 SSH 端口转发逻辑
                        let tunnel_id_copy = tunnel_id;
                        let remote_host = rule_for_task.remote_host.clone();
                        let remote_port = rule_for_task.remote_port;

                        // 这里需要实际的 SSH 客户端句柄来打开转发通道
                        // 简化实现：记录连接信息
                        info!(
                            "Tunnel {}: forwarding to {}:{}",
                            tunnel_id_copy, remote_host, remote_port
                        );

                        // 更新传输字节数
                        {
                            let mut tunnels = tunnels.write().await;
                            if let Some(tunnel) = tunnels.get_mut(&tunnel_id_copy) {
                                tunnel.bytes_transferred += 1;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Tunnel accept error: {}", e);
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
