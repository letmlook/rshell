//! 触发器引擎
//!
//! 基于终端输出自动检测匹配条件并执行动作。
//! 支持正则匹配和精确匹配。

use crate::error::CoreError;
use crate::event_bus::EventBus;
use regex::Regex;
use rshell_api::types::{Trigger, TriggerAction, TriggerCondition};
use rshell_api::AppEvent;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
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
    /// 自触发冷却：会话 → 触发器 → 进入冷却的时刻
    ///
    /// 真实事故：触发器「匹配到 X 就发送文本 Y」，而 Y 本身也含 X ——
    /// 发送出去的 Y 会成为新的输出，再次匹配 X，再次发送 Y …
    /// 真机上实测一条这样的触发器自触发 19081 次把终端刷屏。
    ///
    /// 只有 **SendText** 会造成这种循环（其它动作不写回会话输出），
    /// 所以只有它们进冷却表。
    ///
    /// **冷却是「有界时间窗」而不是「直到用户再次输入」**：远端的回声通常在
    /// 毫秒级回来，但「应答型」触发器（远端反复问密码 → 每次都应答）必须能在
    /// 下一轮提问时再次触发 —— 只认用户输入会让它只应答一次，之后会话卡住。
    /// 所以：进入冷却后 `COOLDOWN_WINDOW` 内不再匹配，窗口一过自动恢复；
    /// 用户在该会话输入也会立刻解除冷却（那是「新一轮」的天然分界）。
    ///
    /// 静态层面已经在 `create_trigger` 拒绝「动作文本匹配自身条件」的配置，
    /// 但远端**对动作的回应**也可能再次匹配条件（例如远端回显同样的文本），
    /// 那种情况静态检查看不出来，只能靠这里的运行时冷却兜住。
    ///
    /// 冷却的进入点在 `check_output` 里（所有协议路径的输出处理都要经过它），
    /// 这样新增协议路径时不会漏掉 —— 之前只在 SSH 路径里加过，
    /// Telnet/Serial 上的自触发循环原样存在。
    self_cooldown: Arc<RwLock<HashMap<Uuid, HashMap<Uuid, Instant>>>>,
}

/// 自触发冷却的时间窗。
///
/// 回声在毫秒级回来，1.5 秒足够挡住「写出去 → 被自己再匹配 → 再写出去」的
/// 闭环；又足够短，不至于让「远端反复提问、每次都要应答」的触发器只应答一次。
const COOLDOWN_WINDOW: Duration = Duration::from_millis(1500);

/// 触发器匹配结果
pub struct TriggerMatch {
    pub trigger_id: Uuid,
    pub action: TriggerAction,
}

/// 动作要发送的文本本身是否匹配它自己的条件
///
/// 只有 **SendText** 会把文本写回会话输出（其它动作不写回），所以只有它
/// 可能形成「写出去 → 被自己再匹配 → 再写出去」的闭环。
///
/// 判定与运行期保持一致：正则用 `is_match`、精确匹配用 `contains`。
/// 非法正则视为不匹配（与运行期「缓存缺失即不匹配」的兜底语义一致）。
fn action_matches_own_condition(trigger: &Trigger) -> bool {
    let TriggerAction::SendText(text) = &trigger.action else {
        return false;
    };
    match &trigger.condition {
        TriggerCondition::RegexAppear(pattern) => Regex::new(pattern)
            .map(|re| re.is_match(text))
            .unwrap_or(false),
        TriggerCondition::ExactMatch(needle) => text.contains(needle.as_str()),
    }
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
            self_cooldown: Arc::new(RwLock::new(HashMap::new())),
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
                        // 加载时也要挡掉「动作文本匹配自身条件」的触发器：
                        // 静态校验只对**新建**生效，旧版本存下来的这类触发器
                        // （或用户手改的文件）重启后照样加载 → 照样自触发。
                        // 这里跳过它并告警，**不改文件**（免得动用户的持久化数据）。
                        if action_matches_own_condition(&item) {
                            warn!(
                                trigger_id = %item.id,
                                name = %item.name,
                                "Stored trigger's action text matches its own condition; skipping it"
                            );
                            continue;
                        }
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
            self_cooldown: Arc::new(RwLock::new(HashMap::new())),
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
        crate::storage::persist_json(path, &bytes)
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
        // 自触发防护（创建时）：动作要发送的文本本身若就匹配自己的条件，
        // 这条触发器**可能**无限循环 —— 发出去的文本会变成新的输出，
        // 再次匹配、再次发送。真机上实测过一条这样的触发器自触发 19081 次。
        // 与其让它在运行期疯狂刷屏，不如在这里明确拒绝并说明原因。
        //
        // 注意用词是「可能」而不是「必然」：运行期匹配的是**整块输出**，
        // 动作文本只是其中一部分 —— 远端若不��显（read -s、不回显的脚本），
        // 它并不会真的循环；反过来，远端**回显**了别的含条件的文本时，
        // 静态检查也看不出来（那种情况由运行期冷却兜住）。
        if action_matches_own_condition(&trigger) {
            return Err(CoreError::InvalidState(
                "Trigger action text matches its own condition; if the remote echoes it, \
                 the trigger would keep firing itself"
                    .into(),
            ));
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
        // 冷却表里也摘掉它，否则每个会话都留下一条永远不会被清理的条目
        self.forget_trigger(trigger_id);

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
    ///
    /// **自触发冷却**：处于冷却中的触发器本轮不参与匹配 ——
    /// 它刚刚自己写出去过文本，那段文本会变成新的输出再次匹配它，
    /// 不挡住就是无限循环（真机实测曾自触发 19081 次把终端刷屏）。
    ///
    /// **匹配成功的 SendText 触发器立刻进入冷却**（在返回结果之前）——
    /// 冷却必须紧跟匹配：动作写出去的文本会立刻变成新的输出再次匹配它。
    /// 放在这里而不是各个协议路径里，是因为**所有**协议路径的输出处理都要
    /// 经过这个函数；此前只在 SSH 路径里加过冷却，Telnet/Serial 上的
    /// 自触发循环原样存在。
    pub fn check_output(
        &self,
        output: &str,
        session_id: Uuid,
    ) -> Result<Vec<TriggerMatch>, CoreError> {
        let triggers = self
            .triggers
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        let regex_cache = self
            .regex_cache
            .read()
            .map_err(|e| CoreError::Internal(e.to_string()))?;
        // 冷却表读锁只取一次（热路径：每块输出要遍历全部触发器）。
        //
        // **读锁不能持有到函数结束**：下面 `enter_cooldown` 要拿同一把锁的写锁，
        // 「持读锁再要写锁」会死锁（std RwLock 不可升级）。所以在这里就把
        // 冷却集合算出来、立刻放掉读锁。
        let now = Instant::now();
        let cooling: HashSet<Uuid> = match self.self_cooldown.read() {
            Ok(map) => map
                .get(&session_id)
                .map(|triggers| {
                    triggers
                        .iter()
                        .filter(|(_, entered_at)| {
                            now.duration_since(**entered_at) < COOLDOWN_WINDOW
                        })
                        .map(|(id, _)| *id)
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => {
                // 锁中毒：宁可全部当成冷却，也不要静默失去自触发保护
                warn!(
                    "self-trigger cooldown lock poisoned; treating every trigger as cooling down"
                );
                triggers.keys().copied().collect()
            }
        };
        let mut matches = Vec::new();

        for trigger in triggers.values() {
            if !trigger.enabled {
                continue;
            }
            // 自触发冷却中：它刚刚自己写出去过文本，那段输出会再次匹配它
            if cooling.contains(&trigger.id) {
                debug!(trigger_id = %trigger.id, session_id = %session_id, "Trigger in self-trigger cooldown");
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

        // 命中的 SendText 触发器立刻进入冷却（见文档说明）
        // —— 先冷却、再由调用方执行动作，顺序不能反。
        for m in &matches {
            if matches!(m.action, TriggerAction::SendText(_)) {
                self.enter_cooldown(session_id, m.trigger_id);
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

    // ===== 自触发冷却 =====

    /// 该触发器当前是否处于自触发冷却（本次不再匹配）
    ///
    /// 冷却是**有界时间窗**：进入冷却 `COOLDOWN_WINDOW` 后自动失效，
    /// 这样「应答型」触发器（远端反复提问、每次都要应答）在下一轮提问时
    /// 仍能再次触发，而不是被永久压住。
    pub fn is_cooling_down(&self, session_id: Uuid, trigger_id: Uuid) -> bool {
        let now = Instant::now();
        match self.self_cooldown.read() {
            Ok(map) => map
                .get(&session_id)
                .and_then(|triggers| triggers.get(&trigger_id))
                .is_some_and(|entered_at| now.duration_since(*entered_at) < COOLDOWN_WINDOW),
            Err(_) => {
                // 锁中毒：宁可当成「还在冷却」，也不要静默失去自触发保护
                warn!("self-trigger cooldown lock poisoned; treating trigger as cooling down");
                true
            }
        }
    }

    /// 触发器触发后进入自触发冷却
    ///
    /// 同时清掉该会话里**已过期**的条目 —— 冷却表是长期存活的结构，
    /// 不清理会随会话/触发器数量单调增长。
    pub fn enter_cooldown(&self, session_id: Uuid, trigger_id: Uuid) {
        let now = Instant::now();
        match self.self_cooldown.write() {
            Ok(mut map) => {
                let entry = map.entry(session_id).or_default();
                entry.retain(|_, entered_at| now.duration_since(*entered_at) < COOLDOWN_WINDOW);
                entry.insert(trigger_id, now);
                if entry.is_empty() {
                    map.remove(&session_id);
                }
            }
            Err(_) => warn!("self-trigger cooldown lock poisoned; cooldown not recorded"),
        }
    }

    /// 用户在该会话输入了新内容：解除该会话所有触发器的冷却
    ///
    /// 用户的输入是「新一轮」的天然分界 —— 之后的输出应当重新参与匹配。
    pub fn clear_cooldown(&self, session_id: Uuid) {
        if let Ok(mut map) = self.self_cooldown.write() {
            map.remove(&session_id);
        }
    }

    /// 触发器被删除：把它从所有会话的冷却表里摘掉
    ///
    /// 不摘的话，它在每个会话里都留下一条永远不会被清理的条目。
    pub fn forget_trigger(&self, trigger_id: Uuid) {
        if let Ok(mut map) = self.self_cooldown.write() {
            for triggers in map.values_mut() {
                triggers.remove(&trigger_id);
            }
            map.retain(|_, triggers| !triggers.is_empty());
        }
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

    // ===== 自触发防护 =====

    /// 自匹配配置必须在创建时被拒绝：
    /// 「匹配到 X 就发送 Y」而 Y 本身含 X —— 发出去的 Y 会变成新的输出，
    /// 再次匹配 X，再次发送 Y …（真机实测自触发 19081 次把终端刷屏）
    #[test]
    fn send_text_trigger_matching_its_own_condition_is_rejected() {
        let eng = make_engine();
        let err = eng
            .create_trigger(make_trigger(
                "loop",
                TriggerCondition::RegexAppear("RSHELL_TRIGGER_".to_string()),
                TriggerAction::SendText("echo RSHELL_TRIGGER_FIRED".to_string()),
            ))
            .expect_err("self-matching trigger must be rejected");
        assert!(
            format!("{err}").contains("matches its own condition"),
            "错误信息要说明原因: {err}"
        );
    }

    /// 精确匹配条件下同样要拒绝
    #[test]
    fn send_text_trigger_matching_own_exact_condition_is_rejected() {
        let eng = make_engine();
        let err = eng
            .create_trigger(make_trigger(
                "loop",
                TriggerCondition::ExactMatch("LOOP".to_string()),
                TriggerAction::SendText("echo LOOP".to_string()),
            ))
            .expect_err("self-matching trigger must be rejected");
        assert!(format!("{err}").contains("matches its own condition"));
    }

    /// 不自匹配的 SendText 触发器照常创建
    #[test]
    fn send_text_trigger_not_matching_its_own_condition_is_accepted() {
        let eng = make_engine();
        eng.create_trigger(make_trigger(
            "ok",
            TriggerCondition::ExactMatch("prompt>".to_string()),
            TriggerAction::SendText("yes".to_string()),
        ))
        .expect("non-self-matching trigger must be accepted");
    }

    /// 冷却中的触发器不再匹配（挡住自触发循环）
    ///
    /// 冷却由 `check_output` **自己**进入 —— 所有协议路径（SSH / Telnet /
    /// Serial）的输出处理都经过它，所以这里测的就是生产路径：
    /// 此前只在 SSH 路径里加过冷却，Telnet/Serial 上的自触发循环原样存在。
    #[test]
    fn cooled_down_trigger_does_not_match_again() {
        let eng = make_engine();
        let t = make_trigger(
            "loop",
            TriggerCondition::ExactMatch("X".to_string()),
            TriggerAction::SendText("Y".to_string()),
        );
        let id = eng.create_trigger(t.clone()).unwrap();
        let sid = Uuid::new_v4();

        // 第一次匹配
        assert_eq!(eng.check_output("X", sid).unwrap().len(), 1);

        // 匹配成功后自动进入冷却：动作写出去的文本再次匹配时不再触发
        assert!(eng.is_cooling_down(sid, id));
        assert!(
            eng.check_output("X", sid).unwrap().is_empty(),
            "冷却中的触发器不得再次匹配（否则就是无限循环）"
        );
    }

    /// 冷却是**有界时间窗**：窗口一过自动恢复
    ///
    /// 「应答型」触发器（远端反复问密码 → 每次都应答）必须能在下一轮提问时
    /// 再次触发 —— 早期实现只认「用户再次输入」才重新武装，那会让它只应答
    /// 一次，之后会话卡在等待输入。
    #[test]
    fn cooldown_expires_after_the_window() {
        let eng = make_engine();
        let t = make_trigger(
            "ask",
            TriggerCondition::ExactMatch("Password:".to_string()),
            TriggerAction::SendText("secret".to_string()),
        );
        let id = eng.create_trigger(t).unwrap();
        let sid = Uuid::new_v4();

        assert_eq!(eng.check_output("Password:", sid).unwrap().len(), 1);
        assert!(eng.check_output("Password:", sid).unwrap().is_empty());

        // 把冷却时刻改成「时间窗之前」—— 模拟窗口已过
        {
            let mut map = eng.self_cooldown.write().unwrap();
            let entry = map.get_mut(&sid).expect("冷却表里应有这条会话");
            entry.insert(
                id,
                Instant::now() - COOLDOWN_WINDOW - Duration::from_millis(1),
            );
        }
        assert_eq!(
            eng.check_output("Password:", sid).unwrap().len(),
            1,
            "冷却时间窗过去后触发器必须能再次应答（否则会话会卡在等待输入）"
        );
    }

    /// 旧版本存下来的自匹配触发器，重启后要被跳过
    ///
    /// 静态校验只对**新建**生效；`triggers.json` 里已有的自匹配触发器
    /// （旧版本存的、或用户手改的）加载时若不挡，就会照样自触发。
    /// 加载时跳过它并告警，**但不改写用户的文件**。
    #[test]
    fn stored_self_matching_trigger_is_skipped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("triggers.json");
        let bad = make_trigger(
            "loop",
            TriggerCondition::ExactMatch("LOOP".to_string()),
            TriggerAction::SendText("echo LOOP".to_string()),
        );
        let good = make_trigger(
            "ok",
            TriggerCondition::ExactMatch("prompt>".to_string()),
            TriggerAction::ShowNotification("hi".to_string()),
        );
        std::fs::write(
            &path,
            serde_json::to_vec(&vec![bad.clone(), good.clone()]).unwrap(),
        )
        .unwrap();

        let eng = TriggerEngine::with_path(Arc::new(EventBus::new()), path.clone());
        let sid = Uuid::new_v4();
        assert!(
            eng.check_output("LOOP", sid).unwrap().is_empty(),
            "自匹配的存量触发器不得参与匹配"
        );
        assert_eq!(
            eng.check_output("prompt>", sid).unwrap().len(),
            1,
            "其它触发器照常工作"
        );

        let reloaded: Vec<Trigger> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(reloaded.len(), 2, "加载时跳过不等于改写用户的持久化文件");
    }

    /// 用户再次输入后解除冷却，触发器重新武装
    #[test]
    fn user_input_clears_the_cooldown() {
        let eng = make_engine();
        let t = make_trigger(
            "loop",
            TriggerCondition::ExactMatch("X".to_string()),
            TriggerAction::SendText("Y".to_string()),
        );
        let id = eng.create_trigger(t.clone()).unwrap();
        let sid = Uuid::new_v4();

        eng.enter_cooldown(sid, id);
        assert!(eng.check_output("X", sid).unwrap().is_empty());

        eng.clear_cooldown(sid);
        assert_eq!(
            eng.check_output("X", sid).unwrap().len(),
            1,
            "用户输入后触发器应重新武装"
        );
    }

    /// 冷却只作用于该会话：另一个会话的同名触发器不受影响
    #[test]
    fn cooldown_is_scoped_to_one_session() {
        let eng = make_engine();
        let t = make_trigger(
            "loop",
            TriggerCondition::ExactMatch("X".to_string()),
            TriggerAction::SendText("Y".to_string()),
        );
        let id = eng.create_trigger(t.clone()).unwrap();
        let hot = Uuid::new_v4();
        let other = Uuid::new_v4();

        eng.enter_cooldown(hot, id);
        assert!(eng.check_output("X", hot).unwrap().is_empty());
        assert_eq!(
            eng.check_output("X", other).unwrap().len(),
            1,
            "冷却不得影响其它会话"
        );
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
