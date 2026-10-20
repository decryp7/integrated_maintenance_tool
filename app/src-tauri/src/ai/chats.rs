//! The chats of one window. Each chat is locked only by its own running turn, so listing, reading history,
//! replaying events and cancelling stay responsive while the model answers; a cached record of every chat
//! serves the UI and the project/session autosave without waiting for a turn to end.

use imt_ai::chat::{ChatHistoryRecord, ChatRecord, ChatSession, Envelope, CHAT_RECORD_VERSION, REPLAY_LIMIT};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// More chats than this per window are refused (oldest must be deleted first).
pub const MAX_SESSIONS: usize = 50;
/// Turns kept per chat when a saved history is restored (the newest).
pub const MAX_RESTORED_TURNS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub turns: usize,
    pub running: bool,
}

struct Slot {
    id: String,
    session: Arc<Mutex<ChatSession>>,
    /// The running turn's cancel flag.
    running: Option<Arc<AtomicBool>>,
    record: ChatRecord,
    /// Events of the turns of this run, for a UI that reconnects (mirrors the session's own replay).
    replay: VecDeque<Envelope>,
}

#[derive(Default)]
struct Inner {
    slots: Vec<Slot>,
    counter: u64,
}

/// A turn that was started: the session to run it on and its cancel flag.
pub struct TurnHandle {
    pub session: Arc<Mutex<ChatSession>>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct WindowChats {
    inner: Mutex<Inner>,
}

fn empty_record(id: &str) -> ChatRecord {
    ChatSession::new(id).to_record()
}

impl WindowChats {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        crate::sync::lock(&self.inner)
    }

    /// A new, empty chat; returns its id.
    pub fn create(&self) -> Result<String, String> {
        let mut inner = self.lock();
        if inner.slots.len() >= MAX_SESSIONS {
            return Err(format!("A window holds at most {MAX_SESSIONS} chats; delete one first."));
        }
        let mut id;
        loop {
            inner.counter += 1;
            id = format!("chat-{}", inner.counter);
            if !inner.slots.iter().any(|s| s.id == id) {
                break;
            }
        }
        inner.slots.push(Slot {
            id: id.clone(),
            session: Arc::new(Mutex::new(ChatSession::new(id.clone()))),
            running: None,
            record: empty_record(&id),
            replay: VecDeque::new(),
        });
        Ok(id)
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.lock()
            .slots
            .iter()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                title: s.record.title.clone(),
                turns: s.record.turns.len(),
                running: s.running.is_some(),
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<ChatRecord> {
        self.lock().slots.iter().find(|s| s.id == id).map(|s| s.record.clone())
    }

    /// Removes a chat (cancelling its turn). False when there is no such chat.
    pub fn delete(&self, id: &str) -> bool {
        let mut inner = self.lock();
        let Some(i) = inner.slots.iter().position(|s| s.id == id) else {
            return false;
        };
        if let Some(c) = &inner.slots[i].running {
            c.store(true, Ordering::Relaxed);
        }
        inner.slots.remove(i);
        true
    }

    /// Marks a turn as running. Fails when the chat does not exist or already answers.
    pub fn begin_turn(&self, id: &str) -> Result<TurnHandle, String> {
        let mut inner = self.lock();
        let slot = inner.slots.iter_mut().find(|s| s.id == id).ok_or("This chat no longer exists.")?;
        if slot.running.is_some() {
            return Err("This chat is still answering; wait for it or stop it first.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        slot.running = Some(cancel.clone());
        Ok(TurnHandle {
            session: slot.session.clone(),
            cancel,
        })
    }

    /// Records a streamed event for `events_since`.
    pub fn push_event(&self, id: &str, env: &Envelope) {
        if let Some(slot) = self.lock().slots.iter_mut().find(|s| s.id == id) {
            slot.replay.push_back(env.clone());
            while slot.replay.len() > REPLAY_LIMIT {
                slot.replay.pop_front();
            }
        }
    }

    /// The turn ended: the chat is free again and its saved record is updated.
    pub fn end_turn(&self, id: &str, record: ChatRecord) {
        if let Some(slot) = self.lock().slots.iter_mut().find(|s| s.id == id) {
            slot.running = None;
            slot.record = record;
        }
    }

    /// Stops a running turn. False when nothing runs.
    pub fn cancel(&self, id: &str) -> bool {
        match self.lock().slots.iter().find(|s| s.id == id).and_then(|s| s.running.clone()) {
            Some(c) => {
                c.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Events after `seq`; `None` when the chat is unknown or `seq` is too old to resume from.
    pub fn events_since(&self, id: &str, seq: u64) -> Option<Vec<Envelope>> {
        let inner = self.lock();
        let slot = inner.slots.iter().find(|s| s.id == id)?;
        match slot.replay.front() {
            Some(first) if first.seq > seq + 1 => None,
            _ => Some(slot.replay.iter().filter(|e| e.seq > seq).cloned().collect()),
        }
    }

    /// Everything to save with the project or session.
    pub fn history(&self) -> ChatHistoryRecord {
        ChatHistoryRecord {
            version: CHAT_RECORD_VERSION,
            sessions: self.lock().slots.iter().map(|s| s.record.clone()).collect(),
        }
    }

    /// Replaces every chat with a saved history (running turns are cancelled). Oversized histories are cut.
    pub fn restore(&self, record: ChatHistoryRecord) {
        let mut inner = self.lock();
        for s in &inner.slots {
            if let Some(c) = &s.running {
                c.store(true, Ordering::Relaxed);
            }
        }
        let mut slots: Vec<Slot> = Vec::new();
        for mut r in record.sessions.into_iter().take(MAX_SESSIONS) {
            if r.id.trim().is_empty() || r.id.len() > 64 || slots.iter().any(|s| s.id == r.id) {
                continue;
            }
            if r.turns.len() > MAX_RESTORED_TURNS {
                r.turns.drain(..r.turns.len() - MAX_RESTORED_TURNS);
            }
            slots.push(Slot {
                id: r.id.clone(),
                session: Arc::new(Mutex::new(ChatSession::from_record(r.clone()))),
                running: None,
                record: r,
                replay: VecDeque::new(),
            });
        }
        inner.counter = slots
            .iter()
            .filter_map(|s| s.id.strip_prefix("chat-").and_then(|n| n.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        inner.slots = slots;
    }

    /// Whether `url` is a web source shown in one of these chats (the only links the chat may open in the
    /// system browser).
    pub fn knows_url(&self, url: &str) -> bool {
        use imt_ai::assistant::Citation;
        use imt_ai::chat::ChatEvent;
        let inner = self.lock();
        inner.slots.iter().any(|s| {
            s.record.turns.iter().any(|t| {
                t.web.iter().any(|w| w.url == url) || t.citations.iter().any(|c| matches!(c, Citation::Web { url: u, .. } if u == url))
            }) || s.replay.iter().any(|e| match &e.event {
                ChatEvent::Done { citations, .. } => citations.iter().any(|c| matches!(c, Citation::Web { url: u, .. } if u == url)),
                _ => false,
            })
        })
    }

    /// Cancels every running turn (the window closed).
    pub fn cancel_all(&self) {
        for s in &self.lock().slots {
            if let Some(c) = &s.running {
                c.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_ai::chat::{ChatEvent, TurnRecord};

    fn env(seq: u64) -> Envelope {
        Envelope {
            seq,
            event: ChatEvent::Delta { text: format!("d{seq}") },
        }
    }

    fn turn(user: &str) -> TurnRecord {
        TurnRecord {
            user: user.into(),
            assistant: "a".into(),
            citations: vec![],
            proposals: vec![],
            web: vec![],
            cancelled: false,
            usage: Default::default(),
        }
    }

    #[test]
    fn chats_are_created_listed_run_and_deleted() {
        let c = WindowChats::default();
        let a = c.create().unwrap();
        let b = c.create().unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("chat-1", "chat-2"));
        let t = c.begin_turn(&a).unwrap();
        assert!(c.begin_turn(&a).err().unwrap().contains("still answering"));
        assert!(c.begin_turn("nope").is_err());
        assert!(c.list()[0].running);
        assert!(c.cancel(&a));
        assert!(t.cancel.load(Ordering::Relaxed));
        assert!(!c.cancel(&b));
        let mut r = t.session.lock().unwrap().to_record();
        r.title = "First".into();
        r.turns.push(turn("q"));
        c.end_turn(&a, r);
        let l = c.list();
        assert_eq!((l[0].title.as_str(), l[0].turns, l[0].running), ("First", 1, false));
        assert_eq!(c.get(&a).unwrap().turns.len(), 1);
        assert!(c.get("x").is_none());
        // Deleting a running chat cancels its turn.
        let t = c.begin_turn(&b).unwrap();
        assert!(c.delete(&b));
        assert!(t.cancel.load(Ordering::Relaxed));
        assert!(!c.delete(&b));
        // Ids are not reused.
        assert_eq!(c.create().unwrap(), "chat-3");
        let t = c.begin_turn("chat-3").unwrap();
        c.cancel_all();
        assert!(t.cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn only_links_shown_in_answers_are_known() {
        use imt_ai::assistant::{Citation, WebSource};
        let c = WindowChats::default();
        let id = c.create().unwrap();
        assert!(!c.knows_url("https://a.example/"));
        let mut r = c.get(&id).unwrap();
        let mut t = turn("q");
        t.web.push(WebSource {
            title: "A".into(),
            url: "https://a.example/".into(),
        });
        t.citations.push(Citation::Web {
            title: "B".into(),
            url: "https://b.example/".into(),
        });
        r.turns.push(t);
        c.end_turn(&id, r);
        c.push_event(
            &id,
            &Envelope {
                seq: 2,
                event: ChatEvent::Done {
                    text: String::new(),
                    citations: vec![Citation::Web {
                        title: "C".into(),
                        url: "https://c.example/".into(),
                    }],
                    usage: Default::default(),
                    notes: vec![],
                },
            },
        );
        c.push_event(&id, &env(3));
        for u in ["https://a.example/", "https://b.example/", "https://c.example/"] {
            assert!(c.knows_url(u), "{u}");
        }
        assert!(!c.knows_url("https://evil.example/"));
    }

    #[test]
    fn the_number_of_chats_is_bounded() {
        let c = WindowChats::default();
        for _ in 0..MAX_SESSIONS {
            c.create().unwrap();
        }
        assert!(c.create().unwrap_err().contains("at most"));
    }

    #[test]
    fn events_replay_from_a_sequence_number() {
        let c = WindowChats::default();
        let id = c.create().unwrap();
        assert!(c.events_since(&id, 0).unwrap().is_empty());
        for s in 1..=3 {
            c.push_event(&id, &env(s));
        }
        c.push_event("nope", &env(9));
        let seqs = |v: Vec<Envelope>| v.iter().map(|e| e.seq).collect::<Vec<_>>();
        assert_eq!(seqs(c.events_since(&id, 1).unwrap()), [2, 3]);
        assert_eq!(seqs(c.events_since(&id, 3).unwrap()), Vec::<u64>::new());
        assert!(c.events_since("nope", 0).is_none());
        for s in 4..=(REPLAY_LIMIT as u64 + 10) {
            c.push_event(&id, &env(s));
        }
        assert!(c.events_since(&id, 1).is_none(), "too old to resume");
        assert!(c.events_since(&id, REPLAY_LIMIT as u64).is_some());
    }

    #[test]
    fn history_round_trips_and_restoring_is_bounded() {
        let c = WindowChats::default();
        let id = c.create().unwrap();
        let mut r = c.get(&id).unwrap();
        r.title = "T".into();
        r.turns = (0..3).map(|i| turn(&format!("q{i}"))).collect();
        c.end_turn(&id, r);
        let h = c.history();
        assert_eq!(h.version, CHAT_RECORD_VERSION);

        let other = WindowChats::default();
        let running = other.create().unwrap();
        let t = other.begin_turn(&running).unwrap();
        other.restore(h.clone());
        assert!(t.cancel.load(Ordering::Relaxed), "restoring cancels running turns");
        assert_eq!(other.history(), h);
        assert_eq!(other.create().unwrap(), "chat-2");

        let mut big = h.clone();
        big.sessions[0].turns = (0..MAX_RESTORED_TURNS + 5).map(|i| turn(&format!("q{i}"))).collect();
        let mut dup = big.sessions[0].clone();
        dup.turns.clear();
        big.sessions.push(dup);
        let mut blank = big.sessions[0].clone();
        blank.id = " ".into();
        big.sessions.push(blank);
        for i in 0..MAX_SESSIONS + 3 {
            let mut s = h.sessions[0].clone();
            s.id = format!("x{i}");
            big.sessions.push(s);
        }
        other.restore(big);
        let l = other.list();
        assert_eq!(
            l.len(),
            MAX_SESSIONS - 2,
            "duplicates and blank ids are skipped, then the cap applies"
        );
        assert_eq!(l[0].turns, MAX_RESTORED_TURNS);
        assert_eq!(other.get(&id).unwrap().turns[0].user, "q5", "the newest turns are kept");
    }
}
