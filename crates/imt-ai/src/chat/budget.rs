//! Fitting a conversation into a small context window (4k-8k tokens): how much room history, grounding and
//! tool results get, which older turns are folded into a running summary, and how that summary reads.

use crate::assistant::ToolLimits;
use serde::{Deserialize, Serialize};

/// Characters per token used for budgeting (conservative for English plus log text).
pub const CHARS_PER_TOKEN: f64 = 3.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatBudget {
    /// Room for the recent turns kept verbatim.
    pub history_chars: usize,
    /// Room for the grounding block describing what the user is looking at.
    pub context_chars: usize,
    pub tool_limits: ToolLimits,
    /// The running summary of older turns.
    pub summary_chars: usize,
    /// Small windows get fewer tools to leave room for the conversation.
    pub compact_tools: bool,
}

impl ChatBudget {
    /// `tools_chars`: size of the tool descriptions that go into the prompt.
    pub fn new(context_tokens: usize, max_answer_tokens: usize, tools_chars: usize) -> Self {
        let fixed_chars = 900 + tools_chars; // system prompt and rules
        let prompt_chars = ((context_tokens.saturating_sub(max_answer_tokens)) as f64 * CHARS_PER_TOKEN) as usize;
        let free = prompt_chars.saturating_sub(fixed_chars).max(600);
        ChatBudget {
            history_chars: free / 4,
            context_chars: (free / 7).clamp(300, 2500),
            tool_limits: ToolLimits::for_context(context_tokens),
            summary_chars: (free / 10).clamp(200, 1500),
            compact_tools: context_tokens < 6000,
        }
    }

    /// A tighter budget for the retry after a context overflow.
    pub fn tightened(self) -> Self {
        ChatBudget {
            history_chars: 0,
            context_chars: self.context_chars / 2,
            tool_limits: ToolLimits {
                max_chars: (self.tool_limits.max_chars / 2).max(600),
                max_items: (self.tool_limits.max_items / 2).max(2),
                max_str: (self.tool_limits.max_str / 2).max(80),
            },
            summary_chars: self.summary_chars / 2,
            compact_tools: true,
        }
    }
}

/// One finished exchange of a chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub user: String,
    pub assistant: String,
}

impl Turn {
    fn chars(&self) -> usize {
        self.user.chars().count() + self.assistant.chars().count()
    }
}

/// How many of the newest turns fit in `history_chars` (always the newest first; a turn is never cut).
pub fn kept_turns(turns: &[Turn], history_chars: usize) -> usize {
    let mut used = 0;
    let mut kept = 0;
    for t in turns.iter().rev() {
        used += t.chars();
        if used > history_chars {
            break;
        }
        kept += 1;
    }
    kept
}

fn clip(s: &str, n: usize) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= n {
        one_line
    } else {
        format!("{}…", one_line.chars().take(n).collect::<String>())
    }
}

/// The older turns folded into short lines, newest kept when the cap is reached.
pub fn running_summary(older: &[Turn], cap: usize) -> String {
    let lines: Vec<String> = older
        .iter()
        .map(|t| format!("- asked: {} / answered: {}", clip(&t.user, 100), clip(&t.assistant, 180)))
        .collect();
    let mut out: Vec<&String> = Vec::new();
    let mut used = 0;
    for l in lines.iter().rev() {
        used += l.chars().count() + 1;
        if used > cap {
            break;
        }
        out.push(l);
    }
    out.reverse();
    out.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(n: usize) -> Turn {
        Turn {
            user: format!("question {n} {}", "q".repeat(50)),
            assistant: format!("answer {n} {}", "a".repeat(150)),
        }
    }

    #[test]
    fn budgets_scale_with_the_window() {
        let small = ChatBudget::new(4096, 512, 2000);
        let big = ChatBudget::new(16384, 1024, 2000);
        assert!(small.compact_tools && !big.compact_tools);
        assert!(big.history_chars > small.history_chars && big.tool_limits.max_chars > small.tool_limits.max_chars);
        assert!(small.history_chars > 0 && small.context_chars >= 300);
        let tiny = ChatBudget::new(512, 512, 5000);
        assert!(tiny.history_chars > 0);
        let t = small.tightened();
        assert_eq!(t.history_chars, 0);
        assert!(t.tool_limits.max_chars < small.tool_limits.max_chars && t.compact_tools);
    }

    #[test]
    fn newest_turns_are_kept_first() {
        let turns: Vec<Turn> = (0..6).map(turn).collect();
        let one = turns[0].chars();
        assert_eq!(kept_turns(&turns, one * 2 + 5), 2);
        assert_eq!(kept_turns(&turns, one * 100), 6);
        assert_eq!(kept_turns(&turns, 10), 0);
        assert_eq!(kept_turns(&[], 100), 0);
    }

    #[test]
    fn summary_is_capped_and_prefers_recent_turns() {
        let turns: Vec<Turn> = (0..10).map(turn).collect();
        let s = running_summary(&turns, 600);
        assert!(s.chars().count() <= 600);
        assert!(s.contains("question 9") && !s.contains("question 0"));
        assert_eq!(running_summary(&[], 100), "");
        let multi = running_summary(
            &[Turn {
                user: "a\n\n b".into(),
                assistant: "c".into(),
            }],
            500,
        );
        assert_eq!(multi, "- asked: a b / answered: c");
    }
}
