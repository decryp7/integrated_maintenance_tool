//! The app's `AgentApprover`: an agent step that needs the user's approval blocks on a channel until the
//! user answers on the card in the chat (`browser_agent_respond`). No answer within the timeout, Stop, a
//! cancelled turn or a deleted chat all mean Deny: nothing happens without an explicit yes.

use imt_ai::browser::{AgentApprover, ApprovalRequest, Decision};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long a card waits for the user before it counts as Deny.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// The user's answer, as the UI sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionDto {
    Allow,
    AllowSite,
    Deny,
}

impl From<DecisionDto> for Decision {
    fn from(d: DecisionDto) -> Decision {
        match d {
            DecisionDto::Allow => Decision::Allow,
            DecisionDto::AllowSite => Decision::AllowSite,
            DecisionDto::Deny => Decision::Deny,
        }
    }
}

/// Window label, chat id, request id (request ids restart at 1 in every turn).
type Key = (String, String, u64);

/// Cards waiting for an answer, app-wide.
pub struct Approvals {
    pending: Mutex<HashMap<Key, SyncSender<Decision>>>,
    timeout: Duration,
}

impl Default for Approvals {
    fn default() -> Self {
        Approvals::with_timeout(APPROVAL_TIMEOUT)
    }
}

impl Approvals {
    pub fn with_timeout(timeout: Duration) -> Self {
        Approvals {
            pending: Mutex::default(),
            timeout,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Key, SyncSender<Decision>>> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Delivers the user's answer. False when nothing waits for it (already answered, timed out, stopped).
    pub fn respond(&self, window: &str, chat: &str, request_id: u64, decision: Decision) -> bool {
        let tx = self.lock().remove(&(window.to_string(), chat.to_string(), request_id));
        tx.is_some_and(|tx| tx.try_send(decision).is_ok())
    }

    /// Denies everything a chat waits for (Stop, its turn was cancelled or the chat deleted).
    pub fn deny_chat(&self, window: &str, chat: &str) {
        let mut pending = self.lock();
        let keys: Vec<Key> = pending.keys().filter(|(w, c, _)| w == window && c == chat).cloned().collect();
        for k in keys {
            if let Some(tx) = pending.remove(&k) {
                let _ = tx.try_send(Decision::Deny);
            }
        }
    }

    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.lock().len()
    }

    fn wait(&self, key: Key) -> Decision {
        let (tx, rx) = sync_channel(1);
        self.lock().insert(key.clone(), tx);
        let decision = rx.recv_timeout(self.timeout).unwrap_or(Decision::Deny);
        self.lock().remove(&key);
        decision
    }
}

/// The approver of one chat turn.
pub struct ChatApprover {
    pub approvals: Arc<Approvals>,
    pub window: String,
    pub chat: String,
}

impl AgentApprover for ChatApprover {
    fn approve(&self, req: &ApprovalRequest) -> Decision {
        self.approvals.wait((self.window.clone(), self.chat.clone(), req.request_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_ai::browser::{Ask, AskKind};
    use std::time::Instant;

    fn req(id: u64) -> ApprovalRequest {
        ApprovalRequest {
            request_id: id,
            ask: Ask {
                kind: AskKind::NewSite,
                site: None,
                url: Some("https://e.org/".into()),
                element: None,
                text: None,
                submit: false,
                can_allow_site: true,
            },
            url_segments: vec![],
            text_segments: vec![],
        }
    }

    fn approver(a: &Arc<Approvals>, chat: &str) -> ChatApprover {
        ChatApprover {
            approvals: a.clone(),
            window: "main".into(),
            chat: chat.into(),
        }
    }

    fn answer_when_waiting(a: &Arc<Approvals>, chat: &'static str, id: u64, d: Decision) -> std::thread::JoinHandle<bool> {
        let a = a.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            while a.waiting() == 0 && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(5));
            }
            a.respond("main", chat, id, d)
        })
    }

    #[test]
    fn answers_reach_the_waiting_step() {
        let a = Arc::new(Approvals::with_timeout(Duration::from_secs(5)));
        for d in [Decision::Allow, Decision::AllowSite, Decision::Deny] {
            let t = answer_when_waiting(&a, "chat-1", 1, d);
            assert_eq!(approver(&a, "chat-1").approve(&req(1)), d);
            assert!(t.join().unwrap());
            assert_eq!(a.waiting(), 0);
        }
    }

    #[test]
    fn no_answer_in_time_is_deny_and_late_answers_find_nothing() {
        let a = Arc::new(Approvals::with_timeout(Duration::from_millis(30)));
        let start = Instant::now();
        assert_eq!(approver(&a, "chat-1").approve(&req(1)), Decision::Deny);
        assert!(start.elapsed() >= Duration::from_millis(30));
        assert!(!a.respond("main", "chat-1", 1, Decision::Allow));
    }

    #[test]
    fn answers_go_to_the_right_chat_and_stop_denies() {
        let a = Arc::new(Approvals::with_timeout(Duration::from_secs(5)));
        assert!(!a.respond("main", "chat-9", 1, Decision::Allow));
        let a2 = a.clone();
        let t = std::thread::spawn(move || approver(&a2, "chat-2").approve(&req(4)));
        while a.waiting() == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!a.respond("main", "chat-2", 5, Decision::Allow));
        assert!(!a.respond("w-2", "chat-2", 4, Decision::Allow));
        a.deny_chat("main", "other");
        assert_eq!(a.waiting(), 1);
        a.deny_chat("main", "chat-2");
        assert_eq!(t.join().unwrap(), Decision::Deny);
    }

    #[test]
    fn ui_decisions_are_parsed() {
        let p = |v: serde_json::Value| serde_json::from_value::<DecisionDto>(v).map(Decision::from);
        assert_eq!(p(serde_json::json!("allow")).unwrap(), Decision::Allow);
        assert_eq!(p(serde_json::json!("allowSite")).unwrap(), Decision::AllowSite);
        assert_eq!(p(serde_json::json!("deny")).unwrap(), Decision::Deny);
        assert!(p(serde_json::json!("yes")).is_err());
    }
}
