//! 触发器引擎
//!
//! 基于终端输出自动检测匹配条件并执行动作。
//! 支持正则匹配和精确匹配。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use regex::Regex;
use rshell_api::types::{Trigger, TriggerAction, TriggerCondition};
use rshell_api::AppEvent;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// 触发器引擎
pub struct TriggerEngine {
    /// 触发器存储
    triggers: Arc<RwLock<HashMap<Uuid, Trigger>>>,
    /// R2-18：按 trigger_id 预编译的正则缓存。创建/加载时编译一次，
    /// check_output 热路径只做 is_match，不再每条输出块重新编译。
    regex_cache: Arc<RwLock<HashMap<Uuid, Regex>>>,
    /// 事件总线
    event_bus: Arc<EventBus>,
    path: Option<PathBuf>,
    read_only: bool,
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
            regex_cache: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            path: None,
            read_only: false,
        }
    }

    pub fn with_path(event_bus: Arc<EventBus>, path: PathBuf) -> Self {
        let (triggers, regex_cache, writable) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Vec<Trigger>>(&bytes) {
                Ok(items) => {
                    // R2-18：加载时预编译全部正则；非法正则不入缓存，
                    // check_output 视为不匹配并告警（保留原有兜底语义）。
                    let mut map = HashMap::new();
                    let mut cache = HashMap::new();
                    for item in items {
                        if let TriggerCondition::RegexAppear(pattern) = &item.condition {
                            match Regex::new(pattern) {
                                Ok(re) => {
                                    cache.insert(item.id, re);
                                }
                                Err(error) => {
                                    warn!(trigger_id = %item.id, %error, "Invalid regex in stored trigger; treated as non-match");
                                }
                            }
                        }
                        map.insert(item.id, item);
                    }
                    (map, cache, true)
                }
                Err(error) => {
                    warn!(path = %path.display(), %error, "Trigger file is invalid; leaving it untouched");
                    (HashMap::new(), HashMap::new(), false)
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (HashMap::new(), HashMap::new(), true)
            }
            Err(error) => {
                warn!(path = %path.display(), %error, "Could not read triggers; leaving file untouched");
                (HashMap::new(), HashMap::new(), false)
            }
        };
        Self {
            triggers: Arc::new(RwLock::new(triggers)),
            regex_cache: Arc::new(RwLock::new(regex_cache)),
            event_bus,
            path: Some(path),
            read_only: !writable,
        }
    }

    fn persist(&self, triggers: &HashMap<Uuid, Trigger>) -> Result<(), CoreError> {
        if self.read_only {
            return Err(CoreError::StorageError(
                "Trigger file could not be read; preserving it without changes".into(),
            ));
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let items: Vec<_> = triggers.values().cloned().collect();
        let bytes = serde_json::to_vec_pretty(&items)
            .map_err(|e| CoreError::StorageError(e.to_string()))?;
        crate::script::quick_command::persist_json(path, &bytes)
    }

    /// 创建触发器
    pub fn create_trigger(&self, trigger: Trigger) -> Result<Uuid, CoreError> {
        // R2-18：校验时编译一次并保留，落库成功后写入缓存
        let compiled =
            match &trigger.condition {
                TriggerCondition::RegexAppear(pattern) => {
                    if pattern.is_empty() {
                        return Err(CoreError::InvalidState(
                            "Trigger regex cannot be empty".into(),
                        ));
                    }
                    Some(Regex::new(pattern).map_err(|e| {
                        CoreError::InvalidState(format!("Invalid trigger regex: {e}"))
                    })?)
                }
                TriggerCondition::ExactMatch(text) if text.is_empty() => {
                    return Err(CoreError::InvalidState(
                        "Trigger match text cannot be empty".into(),
                    ));
                }
                _ => None,
            };
        match &trigger.action {
            TriggerAction::SendText(text) | TriggerAction::ShowNotification(text)
                if text.is_empty() =>
            {
                return Err(CoreError::InvalidState(
                    "Trigger action text cannot be empty".into(),
                ));
            }
            TriggerAction::LogToFile(path) if !path.is_absolute() || path.file_name().is_none() => {
                return Err(CoreError::InvalidState(
                    "Trigger log path must be an absolute file".into(),
                ));
            }
            _ => {}
        }
        let id = trigger.id;
        info!(trigger_id = %id, name = %trigger.name, "Creating trigger");

        let mut triggers = self
            .triggers
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        let mut updated = triggers.clone();
        updated.insert(id, trigger);
        self.persist(&updated)?;
        *triggers = updated;
        drop(triggers);
        if let Some(re) = compiled {
            self.regex_cache
                .write()
                .map_err(|e| CoreError::Internal(e.to_string()))?
                .insert(id, re);
        }

        self.event_bus.publish(AppEvent::TriggerListChanged);
        debug!(trigger_id = %id, "Trigger created");
        Ok(id)
    }

    /// 删除触发器
    pub fn delete_trigger(&self, trigger_id: Uuid) -> Result<(), CoreError> {
        info!(trigger_id = %trigger_id, "Deleting trigger");

        let mut triggers = self
            .triggers
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        if !triggers.contains_key(&trigger_id) {
            warn!(trigger_id = %trigger_id, "Trigger not found");
            return Err(CoreError::NotFound(format!(
                "Trigger {} not found",
                trigger_id
            )));
        }
        let mut updated = triggers.clone();
        updated.remove(&trigger_id);
        self.persist(&updated)?;
        *triggers = updated;
        drop(triggers);
        self.regex_cache
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?
            .remove(&trigger_id);

        self.event_bus.publish(AppEvent::TriggerListChanged);
        debug!(trigger_id = %trigger_id, "Trigger deleted");
        Ok(())
    }

    /// 切换触发器启用/禁用
    pub fn toggle_trigger(&self, trigger_id: Uuid) -> Result<(), CoreError> {
        let mut triggers = self
            .triggers
            .write()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        if triggers.contains_key(&trigger_id) {
            let mut updated = triggers.clone();
            let trigger = updated.get_mut(&trigger_id).expect("checked trigger");
            trigger.enabled = !trigger.enabled;
            let enabled = trigger.enabled;
            self.persist(&updated)?;
            *triggers = updated;
            drop(triggers);
            info!(trigger_id = %trigger_id, enabled, "Trigger toggled");
            self.event_bus.publish(AppEvent::TriggerListChanged);
            Ok(())
        } else {
            Err(CoreError::NotFound(format!(
                "Trigger {} not found",
                trigger_id
            )))
        }
    }

    /// 获取所有触发器
    pub fn list_triggers(&self) -> Result<Vec<Trigger>, CoreError> {
        let triggers = self
            .triggers
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        Ok(triggers.values().cloned().collect())
    }

    /// 检查终端输出是否匹配任何触发器
    ///
    /// 返回所有匹配的触发器动作列表。
    /// R2-18：正则使用创建/加载时预编译的缓存（热路径只做 is_match，
    /// 不再每条输出块重新编译）；缓存缺失（文件加载时的非法正则）视为
    /// 不匹配并告警。
    pub fn check_output(
        &self,
        output: &str,
        _session_id: Uuid,
    ) -> Result<Vec<TriggerMatch>, CoreError> {
        let triggers = self
            .triggers
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        let regex_cache = self
            .regex_cache
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        let mut matches = Vec::new();

        for trigger in triggers.values() {
            if !trigger.enabled {
                continue;
            }

            let matched = match &trigger.condition {
                TriggerCondition::RegexAppear(_) => match regex_cache.get(&trigger.id) {
                    Some(re) => re.is_match(output),
                    None => {
                        warn!(trigger_id = %trigger.id, "Regex not cached (invalid pattern); treated as non-match");
                        false
                    }
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use rshell_api::types::{Trigger, TriggerAction, TriggerCondition};
    use std::sync::Arc;

    fn make_engine() -> TriggerEngine {
        TriggerEngine::new(Arc::new(EventBus::new()))
    }

    fn make_trigger(name: &str, condition: TriggerCondition, action: TriggerAction) -> Trigger {
        Trigger {
            id: Uuid::new_v4(),
            name: name.to_string(),
            enabled: true,
            condition,
            action,
        }
    }

    #[test]
    fn saved_trigger_and_enabled_state_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("triggers.json");
        let bus = Arc::new(EventBus::new());
        let first = TriggerEngine::with_path(bus.clone(), path.clone());
        let id = first
            .create_trigger(make_trigger(
                "notify",
                TriggerCondition::ExactMatch("hello".into()),
                TriggerAction::ShowNotification("hello".into()),
            ))
            .unwrap();
        first.toggle_trigger(id).unwrap();
        let restarted = TriggerEngine::with_path(bus, path);
        let triggers = restarted.list_triggers().unwrap();
        assert_eq!(triggers.len(), 1);
        assert!(!triggers[0].enabled);
    }

    #[test]
    fn test_exact_match_fires() {
        let eng = make_engine();
        let t = make_trigger(
            "login",
            TriggerCondition::ExactMatch("welcome".to_string()),
            TriggerAction::ShowNotification("user logged in".to_string()),
        );
        eng.create_trigger(t.clone()).unwrap();
        let sid = Uuid::new_v4();
        let matches = eng.check_output("hello world welcome back", sid).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].trigger_id, t.id);
    }

    #[test]
    fn test_exact_match_misses() {
        let eng = make_engine();
        eng.create_trigger(make_trigger(
            "login",
            TriggerCondition::ExactMatch("welcome".to_string()),
            TriggerAction::Disconnect,
        ))
        .unwrap();
        let matches = eng.check_output("goodbye", Uuid::new_v4()).unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_regex_match_fires() {
        let eng = make_engine();
        let t = make_trigger(
            "error",
            TriggerCondition::RegexAppear(r"(?i)error\s+\d+".to_string()),
            TriggerAction::ShowNotification("error code".to_string()),
        );
        eng.create_trigger(t.clone()).unwrap();
        let matches = eng
            .check_output("Got error 42 from server", Uuid::new_v4())
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].trigger_id, t.id);
    }

    // ===== R2-18：正则创建/加载时预编译缓存 =====

    /// 从文件加载的触发器（with_path 路径）同样命中预编译缓存
    #[test]
    fn regex_trigger_loaded_from_file_matches_via_precompiled_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("triggers.json");
        let t = make_trigger(
            "re",
            TriggerCondition::RegexAppear(r"(?i)error\s+\d+".to_string()),
            TriggerAction::ShowNotification("error code".to_string()),
        );
        std::fs::write(&path, serde_json::to_vec_pretty(&vec![t]).unwrap()).unwrap();

        let eng = TriggerEngine::with_path(Arc::new(EventBus::new()), path);
        let matches = eng
            .check_output("Got error 42 from server", Uuid::new_v4())
            .unwrap();
        assert_eq!(matches.len(), 1);
    }

    /// 文件里的非法正则不入缓存：永不触发（保留加载路径的兜底语义）
    #[test]
    fn invalid_regex_loaded_from_file_never_fires() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("triggers.json");
        let t = make_trigger(
            "bad",
            TriggerCondition::RegexAppear("(unclosed".to_string()),
            TriggerAction::Disconnect,
        );
        std::fs::write(&path, serde_json::to_vec_pretty(&vec![t]).unwrap()).unwrap();

        let eng = TriggerEngine::with_path(Arc::new(EventBus::new()), path);
        let matches = eng.check_output("anything", Uuid::new_v4()).unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_invalid_regex_is_rejected_before_activation() {
        let eng = make_engine();
        assert!(eng
            .create_trigger(make_trigger(
                "bad",
                // 未闭合的括号,Regex::new 必失败
                TriggerCondition::RegexAppear("(unclosed".to_string()),
                TriggerAction::Disconnect,
            ))
            .is_err());
        let matches = eng.check_output("anything", Uuid::new_v4()).unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_disabled_trigger_does_not_fire() {
        let eng = make_engine();
        let mut t = make_trigger(
            "off",
            TriggerCondition::ExactMatch("x".to_string()),
            TriggerAction::Disconnect,
        );
        t.enabled = false;
        eng.create_trigger(t).unwrap();
        let matches = eng.check_output("xxx", Uuid::new_v4()).unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_toggle_trigger_flips_enabled() {
        let eng = make_engine();
        let t = make_trigger(
            "t",
            TriggerCondition::ExactMatch("x".to_string()),
            TriggerAction::Disconnect,
        );
        let id = eng.create_trigger(t).unwrap();
        eng.toggle_trigger(id).unwrap();
        let listed = eng.list_triggers().unwrap();
        assert!(!listed.iter().find(|t| t.id == id).unwrap().enabled);
        eng.toggle_trigger(id).unwrap();
        let listed = eng.list_triggers().unwrap();
        assert!(listed.iter().find(|t| t.id == id).unwrap().enabled);
    }

    #[test]
    fn test_delete_trigger_removes_it() {
        let eng = make_engine();
        let id = eng
            .create_trigger(make_trigger(
                "d",
                TriggerCondition::ExactMatch("x".to_string()),
                TriggerAction::Disconnect,
            ))
            .unwrap();
        eng.delete_trigger(id).unwrap();
        assert!(eng.list_triggers().unwrap().is_empty());
    }

    #[test]
    fn test_delete_missing_returns_not_found() {
        let eng = make_engine();
        let err = eng.delete_trigger(Uuid::new_v4()).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn test_notify_fired_publishes_event() {
        let bus = Arc::new(EventBus::new());
        let eng = TriggerEngine::new(bus.clone());
        let received = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let r = received.clone();
        bus.subscribe(move |event| {
            if let AppEvent::TriggerFired { action_summary, .. } = event {
                r.lock().unwrap().push(action_summary.clone());
            }
        });
        let tid = Uuid::new_v4();
        let sid = Uuid::new_v4();
        eng.notify_fired(tid, sid, "smoke");
        let msgs = received.lock().unwrap();
        assert_eq!(msgs.as_slice(), &["smoke".to_string()]);
    }
}
