//! 快速命令服务
//!
//! 管理快速命令的 CRUD 和执行。
//! 快速命令是常用命令的按钮化封装，支持发送到单个或多个会话。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use rshell_api::types::{QuickCommand, QuickCommandScope};
use rshell_api::AppEvent;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// 快速命令服务
pub struct QuickCommandService {
    /// 命令存储
    commands: Arc<RwLock<HashMap<Uuid, QuickCommand>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
}

impl QuickCommandService {
    /// 创建新的快速命令服务
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            commands: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    /// 创建快速命令
    pub fn create_command(&self, command: QuickCommand) -> Result<Uuid, CoreError> {
        let id = command.id;
        info!(command_id = %id, name = %command.name, "Creating quick command");

        let mut commands = self.commands.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        commands.insert(id, command);

        self.event_bus.publish(AppEvent::QuickCommandListChanged);
        debug!(command_id = %id, "Quick command created");
        Ok(id)
    }

    /// 删除快速命令
    pub fn delete_command(&self, command_id: Uuid) -> Result<(), CoreError> {
        info!(command_id = %command_id, "Deleting quick command");

        let mut commands = self.commands.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        if commands.remove(&command_id).is_none() {
            warn!(command_id = %command_id, "Quick command not found");
            return Err(CoreError::NotFound(format!("Quick command {} not found", command_id)));
        }

        self.event_bus.publish(AppEvent::QuickCommandListChanged);
        debug!(command_id = %command_id, "Quick command deleted");
        Ok(())
    }

    /// 获取所有快速命令
    pub fn list_commands(&self) -> Result<Vec<QuickCommand>, CoreError> {
        let commands = self.commands.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(commands.values().cloned().collect())
    }

    /// 获取指定快速命令
    pub fn get_command(&self, command_id: Uuid) -> Result<Option<QuickCommand>, CoreError> {
        let commands = self.commands.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(commands.get(&command_id).cloned())
    }

    /// 获取快速命令的文本内容（含可选回车）
    pub fn get_command_text(&self, command_id: Uuid) -> Result<Vec<u8>, CoreError> {
        let commands = self.commands.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        let cmd = commands
            .get(&command_id)
            .ok_or_else(|| CoreError::NotFound(format!("Quick command {} not found", command_id)))?;

        let mut text = cmd.command.clone();
        if cmd.send_enter {
            text.push('\n');
        }

        Ok(text.into_bytes())
    }

    /// 获取快速命令的目标会话列表
    pub fn resolve_target_sessions(
        &self,
        command_id: Uuid,
        active_session: Option<Uuid>,
        all_sessions: &[Uuid],
    ) -> Result<Vec<Uuid>, CoreError> {
        let commands = self.commands.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        let cmd = commands
            .get(&command_id)
            .ok_or_else(|| CoreError::NotFound(format!("Quick command {} not found", command_id)))?;

        let targets = match &cmd.scope {
            QuickCommandScope::CurrentSession => {
                if let Some(sid) = active_session {
                    vec![sid]
                } else {
                    return Err(CoreError::Internal("No active session".to_string()));
                }
            }
            QuickCommandScope::AllSessions => all_sessions.to_vec(),
            QuickCommandScope::SelectedSessions(ids) => ids.clone(),
        };

        Ok(targets)
    }
}
