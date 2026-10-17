//! The app's [`Approver`]: a chat turn that wants to search or fetch a page blocks on a channel until the
//! user answers in the chat (`ai_approval_respond`). No answer within the timeout, a cancelled turn or a
//! deleted chat all mean Deny: nothing leaves the machine without an explicit yes.

use imt_ai::web::{ApprovalDecision, ApprovalRequest, Approver};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long a question waits for the user before it counts as Deny.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
/// Longest edited query accepted from the UI (it is redacted again by the engine before sending).
const MAX_EDITED_CHARS: usize = 500;

/// The user's answer, as the UI sends it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DecisionDto {
    Allow,
    AllowEdited { text: String },
    AllowSearchesInChat,
    Deny,
}

impl DecisionDto {
    /// Parses the UI's answer into the library's decision; an empty or oversized edit is refused.
    pub fn into_decision(self) -> Result<ApprovalDecision, String> {
        Ok(match self {
            DecisionDto::Allow => ApprovalDecision::Allow,
            DecisionDto::AllowSearchesInChat => ApprovalDecision::AllowSearchesInChat,
            DecisionDto::Deny => ApprovalDecision::Deny,
            DecisionDto::AllowEdited { text } => {
                let t = text.trim();
                if t.is_empty() {
                    return Err("the edited search is empty".into());
                }
                if t.chars().count() > MAX_EDITED_CHARS {
                    return Err(format!("the edited search is longer than {MAX_EDITED_CHARS} characters"));
                }
                ApprovalDecision::AllowEdited(t.to_string())
            }
        })
    }
}

/// Window label, chat id, request id (request ids restart at 1 in every turn).
type Key = (String, String, u64);

/// Questions waiting for an answer, app-wide.
pub struct Approvals {
    pending: Mutex<HashMap<Key, SyncSender<ApprovalDecision>>>,
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

    fn lock(&self) -> MutexGuard<'_, HashMap<Key, SyncSender<ApprovalDecision>>> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Delivers the user's answer. False when nothing waits for it (already answered, timed out, cancelled).
    pub fn respond(&self, window: &str, session: &str, request_id: u64, decision: ApprovalDecision) -> bool {
        let tx = self.lock().remove(&(window.to_string(), session.to_string(), request_id));
        tx.is_some_and(|tx| tx.try_send(decision).is_ok())
    }

    /// Denies everything a chat waits for (its turn was cancelled or the chat deleted).
    pub fn deny_session(&self, window: &str, session: &str) {
        let mut pending = self.lock();
        let keys: Vec<Key> = pending.keys().filter(|(w, s, _)| w == window && s == session).cloned().collect();
        for k in keys {
            if let Some(tx) = pending.remove(&k) {
                let _ = tx.try_send(ApprovalDecision::Deny);
            }
        }
    }

    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.lock().len()
    }

    fn wait(&self, key: Key) -> ApprovalDecision {
        let (tx, rx) = sync_channel(1);
        self.lock().insert(key.clone(), tx);
        let decision = rx.recv_timeout(self.timeout).unwrap_or(ApprovalDecision::Deny);
        self.lock().remove(&key);
        decision
    }
}

/// The approver of one chat turn.
pub struct SessionApprover {
    pub approvals: Arc<Approvals>,
    pub window: String,
    pub session: String,
}

impl Approver for SessionApprover {
    fn approve(&self, req: &ApprovalRequest) -> ApprovalDecision {
        self.approvals.wait((self.window.clone(), self.session.clone(), req.request_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_ai::web::ApprovalKind;
    use std::time::Instant;

    fn req(id: u64) -> ApprovalRequest {
        ApprovalRequest {
            request_id: id,
            kind: ApprovalKind::Search,
            text: "TimeoutException".into(),
            domain: None,
            provider: "searxng".into(),
            removed: vec![],
        }
    }

    fn approver(a: &Arc<Approvals>, session: &str) -> SessionApprover {
        SessionApprover {
            approvals: a.clone(),
            window: "main".into(),
            session: session.into(),
        }
    }

    /// Answers request `id` of `session` from another thread as soon as it is waiting.
    fn answer_when_waiting(a: &Arc<Approvals>, session: &'static str, id: u64, d: ApprovalDecision) -> std::thread::JoinHandle<bool> {
        let a = a.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            while a.waiting() == 0 && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(5));
            }
            a.respond("main", session, id, d)
        })
    }

    #[test]
    fn allow_edit_and_allow_for_chat_reach_the_waiting_turn() {
        let a = Arc::new(Approvals::with_timeout(Duration::from_secs(5)));
        for d in [
            ApprovalDecision::Allow,
            ApprovalDecision::AllowEdited("other".into()),
            ApprovalDecision::AllowSearchesInChat,
            ApprovalDecision::Deny,
        ] {
            let t = answer_when_waiting(&a, "chat-1", 1, d.clone());
            assert_eq!(approver(&a, "chat-1").approve(&req(1)), d);
            assert!(t.join().unwrap());
            assert_eq!(a.waiting(), 0);
        }
    }

    #[test]
    fn no_answer_in_time_is_deny() {
        let a = Arc::new(Approvals::with_timeout(Duration::from_millis(30)));
        let start = Instant::now();
        assert_eq!(approver(&a, "chat-1").approve(&req(1)), ApprovalDecision::Deny);
        assert!(start.elapsed() >= Duration::from_millis(30));
        assert_eq!(a.waiting(), 0);
        // A late answer finds nothing waiting.
        assert!(!a.respond("main", "chat-1", 1, ApprovalDecision::Allow));
    }

    #[test]
    fn answers_go_to_the_right_chat_and_cancel_denies() {
        let a = Arc::new(Approvals::with_timeout(Duration::from_secs(5)));
        // Wrong chat or request id: not delivered.
        assert!(!a.respond("main", "chat-9", 1, ApprovalDecision::Allow));
        let a2 = a.clone();
        let t = std::thread::spawn(move || approver(&a2, "chat-2").approve(&req(4)));
        while a.waiting() == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!a.respond("main", "chat-2", 5, ApprovalDecision::Allow));
        assert!(!a.respond("w-2", "chat-2", 4, ApprovalDecision::Allow));
        a.deny_session("main", "other");
        assert_eq!(a.waiting(), 1);
        a.deny_session("main", "chat-2");
        assert_eq!(t.join().unwrap(), ApprovalDecision::Deny);
    }

    #[test]
    fn ui_decisions_are_parsed_and_checked() {
        let p = |v: serde_json::Value| serde_json::from_value::<DecisionDto>(v).unwrap().into_decision();
        assert_eq!(p(serde_json::json!({"kind": "allow"})), Ok(ApprovalDecision::Allow));
        assert_eq!(p(serde_json::json!({"kind": "deny"})), Ok(ApprovalDecision::Deny));
        assert_eq!(
            p(serde_json::json!({"kind": "allowSearchesInChat"})),
            Ok(ApprovalDecision::AllowSearchesInChat)
        );
        assert_eq!(
            p(serde_json::json!({"kind": "allowEdited", "text": "  x  "})),
            Ok(ApprovalDecision::AllowEdited("x".into()))
        );
        assert!(p(serde_json::json!({"kind": "allowEdited", "text": " "})).is_err());
        assert!(p(serde_json::json!({"kind": "allowEdited", "text": "a".repeat(501)})).is_err());
        assert!(serde_json::from_value::<DecisionDto>(serde_json::json!({"kind": "yes"})).is_err());
    }
}
