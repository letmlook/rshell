//! 触发器引擎
//!
//! 基于终端输出自动检测匹配条件并执行动作。
//! 支持正则匹配和精确匹配。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use rshell_api::types::{Trigger, TriggerAction, TriggerCondition};
use rshell_api::AppEvent;
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// 触发器引擎
pub struct TriggerEngine {
    /// 触发器存储
    triggers: Arc<RwLock<HashMap<Uuid, Trigger>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
}

/// 触发器匹配结果
pub struct TriggerMatch {
    pub trigger_id: Uuid,
    pub action: TriggerAction,
}

impl TriggerEngine {
    /// 创建新的触发器引擎
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            triggers: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    /// 创建触发器
    pub fn create_trigger(&self, trigger: Trigger) -> Result<Uuid, CoreError> {
        let id = trigger.id;
        info!(trigger_id = %id, name = %trigger.name, "Creating trigger");

        let mut triggers = self.triggers.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        triggers.insert(id, trigger);

        self.event_bus.publish(AppEvent::TriggerListChanged);
        debug!(trigger_id = %id, "Trigger created");
        Ok(id)
    }

    /// 删除触发器
    pub fn delete_trigger(&self, trigger_id: Uuid) -> Result<(), CoreError> {
        info!(trigger_id = %trigger_id, "Deleting trigger");

        let mut triggers = self.triggers.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        if triggers.remove(&trigger_id).is_none() {
            warn!(trigger_id = %trigger_id, "Trigger not found");
            return Err(CoreError::NotFound(format!("Trigger {} not found", trigger_id)));
        }

        self.event_bus.publish(AppEvent::TriggerListChanged);
        debug!(trigger_id = %trigger_id, "Trigger deleted");
        Ok(())
    }

    /// 切换触发器启用/禁用
    pub fn toggle_trigger(&self, trigger_id: Uuid) -> Result<(), CoreError> {
        let mut triggers = self.triggers.write().map_err(|e| CoreError::Internal(e.to_string()))?;
        if let Some(trigger) = triggers.get_mut(&trigger_id) {
            trigger.enabled = !trigger.enabled;
            info!(trigger_id = %trigger_id, enabled = trigger.enabled, "Trigger toggled");
            self.event_bus.publish(AppEvent::TriggerListChanged);
            Ok(())
        } else {
            Err(CoreError::NotFound(format!("Trigger {} not found", trigger_id)))
        }
    }

    /// 获取所有触发器
    pub fn list_triggers(&self) -> Result<Vec<Trigger>, CoreError> {
        let triggers = self.triggers.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(triggers.values().cloned().collect())
    }

    /// 检查终端输出是否匹配任何触发器
    ///
    /// 返回所有匹配的触发器动作列表。
    pub fn check_output(&self, output: &str, _session_id: Uuid) -> Result<Vec<TriggerMatch>, CoreError> {
        let triggers = self.triggers.read().map_err(|e| CoreError::Internal(e.to_string()))?;
        let mut matches = Vec::new();

        for trigger in triggers.values() {
            if !trigger.enabled {
                continue;
            }

            let matched = match &trigger.condition {
                TriggerCondition::RegexAppear(pattern) => {
                    match Regex::new(pattern) {
                        Ok(re) => re.is_match(output),
                        Err(e) => {
                            warn!(trigger_id = %trigger.id, error = %e, "Invalid regex pattern");
                            false
                        }
                    }
                }
                TriggerCondition::ExactMatch(text) => output.contains(text),
            };

            if matched {
                debug!(trigger_id = %trigger.id, "Trigger matched");
                matches.push(TriggerMatch {
                    trigger_id: trigger.id,
                    action: trigger.action.clone(),
                });
            }
        }

        Ok(matches)
    }

    /// 发布触发器触发事件
    pub fn notify_fired(&self, trigger_id: Uuid, session_id: Uuid, action_summary: &str) {
        self.event_bus.publish(AppEvent::TriggerFired {
            trigger_id,
            session_id,
            action_summary: action_summary.to_string(),
        });
    }
}
