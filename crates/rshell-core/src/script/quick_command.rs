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

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::{QuickCommand, QuickCommandScope};
    use std::sync::Arc;

    fn make_svc() -> QuickCommandService {
        QuickCommandService::new(Arc::new(EventBus::new()))
    }

    fn make_cmd(name: &str, text: &str, scope: QuickCommandScope) -> QuickCommand {
        QuickCommand {
            id: Uuid::new_v4(),
            name: name.to_string(),
            command: text.to_string(),
            send_enter: false,
            description: String::new(),
            scope,
            hotkey: None,
            group: None,
        }
    }

    #[test]
    fn test_create_and_list() {
        let svc = make_svc();
        let id = svc
            .create_command(make_cmd("ls", "ls -la", QuickCommandScope::AllSessions))
            .unwrap();
        let all = svc.list_commands().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].name, "ls");
    }

    #[test]
    fn test_get_command_text_without_enter() {
        let svc = make_svc();
        let id = svc
            .create_command(make_cmd("ls", "ls -la", QuickCommandScope::AllSessions))
            .unwrap();
        assert_eq!(svc.get_command_text(id).unwrap(), b"ls -la".to_vec());
    }

    #[test]
    fn test_get_command_text_with_enter() {
        let svc = make_svc();
        let mut cmd = make_cmd("ls", "ls -la", QuickCommandScope::AllSessions);
        cmd.send_enter = true;
        let id = svc.create_command(cmd).unwrap();
        assert_eq!(svc.get_command_text(id).unwrap(), b"ls -la\n".to_vec());
    }

    #[test]
    fn test_resolve_target_sessions_current() {
        let svc = make_svc();
        let id = svc
            .create_command(make_cmd("c", "x", QuickCommandScope::CurrentSession))
            .unwrap();
        let active = Uuid::new_v4();
        let all = vec![Uuid::new_v4(), Uuid::new_v4()];
        let targets = svc.resolve_target_sessions(id, Some(active), &all).unwrap();
        assert_eq!(targets, vec![active]);
    }

    #[test]
    fn test_resolve_target_sessions_current_without_active_errors() {
        let svc = make_svc();
        let id = svc
            .create_command(make_cmd("c", "x", QuickCommandScope::CurrentSession))
            .unwrap();
        let err = svc.resolve_target_sessions(id, None, &[]).unwrap_err();
        assert!(format!("{err}").contains("No active session"));
    }

    #[test]
    fn test_resolve_target_sessions_all() {
        let svc = make_svc();
        let id = svc
            .create_command(make_cmd("c", "x", QuickCommandScope::AllSessions))
            .unwrap();
        let all = vec![Uuid::new_v4(), Uuid::new_v4()];
        let targets = svc.resolve_target_sessions(id, None, &all).unwrap();
        assert_eq!(targets, all);
    }

    #[test]
    fn test_resolve_target_sessions_selected() {
        let svc = make_svc();
        let pick = vec![Uuid::new_v4()];
        let id = svc
            .create_command(make_cmd(
                "c",
                "x",
                QuickCommandScope::SelectedSessions(pick.clone()),
            ))
            .unwrap();
        let targets = svc.resolve_target_sessions(id, None, &[]).unwrap();
        assert_eq!(targets, pick);
    }

    #[test]
    fn test_delete_missing_returns_not_found() {
        let svc = make_svc();
        let err = svc.delete_command(Uuid::new_v4()).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }
}
