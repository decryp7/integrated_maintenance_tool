//! Chat to assist analysis: sessions, per-turn grounding from what the user is looking at, a context budget
//! for small local models, citations, persistence and export. The chat reads and proposes; it never applies.

mod budget;
mod context;
mod host;
mod persist;
mod session;
mod suggest;
mod time;

pub use budget::{kept_turns, running_summary, ChatBudget, Turn, CHARS_PER_TOKEN};
pub use context::{resolve, ChatContext, ResolvedContext, MAX_SELECTED};
pub use host::{propose_specs, ChatHost};
pub use persist::{export_markdown, ChatHistoryRecord, ChatRecord, TurnRecord, CHAT_RECORD_VERSION};
pub use session::{ChatEnv, ChatEvent, ChatSession, ChatSessions, Envelope, REPLAY_LIMIT};
pub use suggest::suggested_prompts;
pub use time::{format_hm, format_utc};
