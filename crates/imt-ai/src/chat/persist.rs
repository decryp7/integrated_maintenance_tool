//! Saving chats with the project. The record is versioned and optional: a project without it loads fine,
//! and a damaged or newer record becomes an empty history with a warning instead of an error.

use super::budget::Turn;
use crate::assistant::{Citation, WebSource};
use crate::llm::Usage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CHAT_RECORD_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRecord {
    pub user: String,
    pub assistant: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
    /// Proposals shown in this turn, as the UI received them (never re-applied automatically).
    #[serde(default)]
    pub proposals: Vec<Value>,
    #[serde(default)]
    pub web: Vec<WebSource>,
    /// The turn was stopped by the user; `assistant` is the partial answer.
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub usage: Usage,
}

impl TurnRecord {
    pub fn as_turn(&self) -> Turn {
        Turn {
            user: self.user.clone(),
            assistant: self.assistant.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRecord {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub turns: Vec<TurnRecord>,
    /// The user allowed web searches for this chat (reset when the project is reopened is also fine).
    #[serde(default)]
    pub allow_searches: bool,
}

/// What goes into the project file under one optional key (for example `"chats"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryRecord {
    pub version: u32,
    #[serde(default)]
    pub sessions: Vec<ChatRecord>,
}

impl Default for ChatHistoryRecord {
    fn default() -> Self {
        ChatHistoryRecord {
            version: CHAT_RECORD_VERSION,
            sessions: vec![],
        }
    }
}

impl ChatHistoryRecord {
    /// Reads the optional project value: `None` is an empty history; a damaged one gives a warning.
    pub fn from_value(v: Option<&Value>) -> (ChatHistoryRecord, Option<String>) {
        let Some(v) = v.filter(|v| !v.is_null()) else {
            return (Self::default(), None);
        };
        match serde_json::from_value::<ChatHistoryRecord>(v.clone()) {
            Ok(r) if r.version <= CHAT_RECORD_VERSION => (r, None),
            Ok(_) => (
                Self::default(),
                Some("The saved chats were written by a newer IMT and were not loaded.".into()),
            ),
            Err(_) => (Self::default(), Some("The saved chats are damaged and were not loaded.".into())),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// The chat as a Markdown document (what was asked, what was answered, the evidence and the sources).
pub fn export_markdown(chat: &ChatRecord) -> String {
    let mut out = format!("# {}\n\n", if chat.title.trim().is_empty() { "Chat" } else { chat.title.trim() });
    for t in &chat.turns {
        out.push_str(&format!(
            "**You:** {}\n\n**Assistant:** {}{}\n\n",
            t.user.trim(),
            t.assistant.trim(),
            if t.cancelled { " *(stopped)*" } else { "" }
        ));
        let ids: Vec<String> = t
            .citations
            .iter()
            .filter_map(|c| {
                if let Citation::Record { id, verified } = c {
                    Some(format!("#{id}{}", if *verified { "" } else { " (unverified)" }))
                } else {
                    None
                }
            })
            .collect();
        if !ids.is_empty() {
            out.push_str(&format!("Evidence: events {}\n\n", ids.join(", ")));
        }
        for c in &t.citations {
            if let Citation::Query { ql } = c {
                out.push_str(&format!("Query: `{ql}`\n\n"));
            }
        }
        let web: Vec<String> = t
            .web
            .iter()
            .map(|w| format!("[{}]({})", w.title.replace(['[', ']'], ""), w.url))
            .collect();
        if !web.is_empty() {
            out.push_str(&format!("Web sources: {}\n\n", web.join(", ")));
        }
        for p in &t.proposals {
            let kind = p.get("kind").and_then(Value::as_str).unwrap_or("proposal");
            out.push_str(&format!("Proposal shown: {kind} (applied only if the user chose Apply)\n\n"));
        }
        out.push_str("---\n\n");
    }
    out
}
