//! Every AI feature reads the knowledge through its model: this decorator of the provider port appends the block to
//! the system prompt of each request, after IMT's own rules, and reports a context window smaller by the block, so
//! every budget computed from it (history, samples, tool output) still fits.

use super::{assemble, budget_chars, KnowledgeBlock, KnowledgeFile};
use crate::error::Result;
use crate::llm::{ChatRequest, ChatResponse, LlmProvider};

/// Characters per token assumed when reserving room for the block (more conservative than the budgets').
const RESERVE_CHARS_PER_TOKEN: usize = 3;

/// A provider whose requests carry the user's knowledge files.
pub struct KnowledgeProvider<P> {
    inner: P,
    block: Option<KnowledgeBlock>,
    reserved: usize,
}

impl<P: LlmProvider> KnowledgeProvider<P> {
    /// `inner` with the enabled `project` and `global` files, within a budget relative to its context window.
    pub fn new(inner: P, project: &[KnowledgeFile], global: &[KnowledgeFile]) -> Self {
        let block = assemble(project, global, budget_chars(inner.context_length()));
        let reserved = block.as_ref().map_or(0, |b| b.text.len().div_ceil(RESERVE_CHARS_PER_TOKEN) + 4);
        KnowledgeProvider { inner, block, reserved }
    }

    /// The block added to every request (none without enabled knowledge).
    pub fn block(&self) -> Option<&KnowledgeBlock> {
        self.block.as_ref()
    }
}

impl<P: LlmProvider> LlmProvider for KnowledgeProvider<P> {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn display_name(&self) -> String {
        self.inner.display_name()
    }

    fn context_length(&self) -> usize {
        self.inner.context_length().saturating_sub(self.reserved)
    }

    fn vision(&self) -> bool {
        self.inner.vision()
    }

    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
        let Some(block) = &self.block else {
            return self.inner.chat(req, on_delta);
        };
        let mut req = req.clone();
        req.system = if req.system.is_empty() {
            block.text.clone()
        } else {
            format!("{}\n\n{}", req.system, block.text)
        };
        self.inner.chat(&req, on_delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ChatMessage, ScriptedProvider};
    use std::sync::Arc;

    fn file(name: &str, text: &str) -> KnowledgeFile {
        KnowledgeFile::new("k1", name, true, text).unwrap()
    }

    #[test]
    fn requests_carry_the_knowledge_after_the_rules() {
        let inner = Arc::new(ScriptedProvider::text(&["one", "two"]));
        let p = KnowledgeProvider::new(inner.clone(), &[file("Glossary", "ACME-GW is the payment gateway.")], &[]);
        assert!(p.context_length() < 8192 && p.context_length() > 7000);
        assert_eq!((p.id(), p.vision()), ("scripted", false));
        assert_eq!(p.display_name(), "Scripted test model");
        let req = |system: &str| ChatRequest {
            system: system.into(),
            messages: vec![ChatMessage::user("what is ACME-GW?")],
            ..ChatRequest::default()
        };
        p.chat(&req("IMT's rules."), &mut |_| {}).unwrap();
        p.chat(&req(""), &mut |_| {}).unwrap();
        let sent = inner.requests();
        assert!(sent[0].system.starts_with("IMT's rules.\n\n<imt_knowledge>"), "{}", sent[0].system);
        assert!(sent[0].system.contains("ACME-GW is the payment gateway."));
        assert!(sent[1].system.starts_with("<imt_knowledge>"));
        assert_eq!(p.block().unwrap().included, 1);
    }

    #[test]
    fn without_enabled_knowledge_requests_pass_unchanged() {
        let inner = Arc::new(ScriptedProvider::text(&["one"]));
        let mut off = file("Off", "x");
        off.enabled = false;
        let p = KnowledgeProvider::new(inner.clone(), &[off], &[]);
        assert!(p.block().is_none());
        assert_eq!(p.context_length(), 8192);
        let req = ChatRequest {
            system: "rules".into(),
            ..ChatRequest::default()
        };
        p.chat(&req, &mut |_| {}).unwrap();
        assert_eq!(inner.requests()[0].system, "rules");
    }

    #[test]
    fn a_small_window_gets_a_cut_block() {
        let inner = Arc::new(ScriptedProvider::text(&["ok"]).with_context(2048));
        let p = KnowledgeProvider::new(inner, &[file("Big", &"fact\n".repeat(4000))], &[]);
        let b = p.block().unwrap();
        assert!(b.text.len() <= budget_chars(2048), "{}", b.text.len());
        assert_eq!(b.cut, ["Big"]);
        assert!(p.context_length() >= 2048 - budget_chars(2048) / 3 - 4);
    }
}
