//! Asking the model for JSON that must pass an engine check. The first invalid answer gets one repair
//! round that quotes the engine's complaint; a second failure is a clear error.

use super::Assistant;
use crate::error::{AiError, Result};
use crate::llm::{ChatMessage, LlmProvider};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The JSON object inside a model answer (which may be wrapped in a code fence or prose).
pub fn extract_json(text: &str) -> &str {
    let t = text.trim();
    match (t.find('{'), t.rfind('}')) {
        (Some(a), Some(b)) if b > a => &t[a..=b],
        _ => t,
    }
}

impl<P: LlmProvider> Assistant<P> {
    /// Returns what `check` builds from the parsed answer, and whether a repair round was needed.
    /// `check` returns `Err(feedback)` to reject an answer; the feedback is shown to the model.
    pub(crate) fn generate_validated<T: DeserializeOwned, R>(
        &self,
        system: &str,
        user: &str,
        schema: Value,
        mut check: impl FnMut(T) -> std::result::Result<R, String>,
    ) -> Result<(R, bool)> {
        let mut messages = vec![ChatMessage::user(user)];
        let mut feedback = String::new();
        for attempt in 0..2 {
            let mut req = self.request(system, messages.clone());
            req.json_schema = Some(schema.clone());
            let resp = self.complete(&req)?;
            let verdict = match serde_json::from_str::<T>(extract_json(&resp.text)) {
                Ok(t) => check(t),
                Err(e) => Err(format!("the answer is not valid JSON of the requested shape ({e})")),
            };
            match verdict {
                Ok(r) => return Ok((r, attempt > 0)),
                Err(f) => {
                    messages.push(ChatMessage::assistant(resp.text));
                    messages.push(ChatMessage::user(format!(
                        "Your answer was rejected: {f}\nReply again with corrected JSON only."
                    )));
                    feedback = f;
                }
            }
        }
        Err(AiError::Malformed(feedback))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_object_from_prose_and_fences() {
        assert_eq!(extract_json("```json\n{\"a\": 1}\n```"), "{\"a\": 1}");
        assert_eq!(extract_json("Sure! {\"a\": {\"b\": 2}} done"), "{\"a\": {\"b\": 2}}");
        assert_eq!(extract_json("no json"), "no json");
        assert_eq!(extract_json("} {"), "} {");
    }
}
