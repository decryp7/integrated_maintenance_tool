//! The AI engine port (ADR 0059): one [`AiEngine`] trait with two adapters, Debrief's built-in llama.cpp worker
//! ([`BuiltInEngine`]) and Pitwall ([`PitwallEngine`]), and the pure choice between them ([`select`]). The features reach
//! the model only through [`AiEngine::llm`], the `LlmProvider` port.

pub mod pitwall;

use crate::llm::{LazyProvider, LlmProvider, LoadState};
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::Arc;

/// Which engine the user wants (Settings ▸ AI ▸ AI engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineChoice {
    /// Pitwall when it is usable, else the built-in engine.
    #[default]
    Auto,
    Pitwall,
    BuiltIn,
}

impl EngineChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineChoice::Auto => "auto",
            EngineChoice::Pitwall => "pitwall",
            EngineChoice::BuiltIn => "builtIn",
        }
    }

    /// Parses a setting value; anything unknown is Automatic (a damaged value never loses the other settings).
    pub fn parse(s: &str) -> EngineChoice {
        match s.trim().to_ascii_lowercase().as_str() {
            "pitwall" => EngineChoice::Pitwall,
            "builtin" | "built-in" | "built_in" => EngineChoice::BuiltIn,
            _ => EngineChoice::Auto,
        }
    }
}

impl<'de> Deserialize<'de> for EngineChoice {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => EngineChoice::parse(&s),
            _ => EngineChoice::Auto,
        })
    }
}

/// The engine kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineKind {
    BuiltIn,
    Pitwall,
}

/// What the choice depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectInput {
    pub choice: EngineChoice,
    /// Pitwall is running, compatible, paired and has a model for Debrief.
    pub pitwall_usable: bool,
    /// Pitwall answered a request of this session.
    pub pitwall_served: bool,
    /// The user switched the session to the built-in engine (the repair "Use the fallback").
    pub built_in_for_session: bool,
}

/// The engine requests go to; `None`: the wanted engine (Pitwall) is not usable and nothing replaces it silently.
pub fn select(i: SelectInput) -> Option<EngineKind> {
    if i.built_in_for_session || i.choice == EngineChoice::BuiltIn {
        return Some(EngineKind::BuiltIn);
    }
    match (i.choice, i.pitwall_usable, i.pitwall_served) {
        (_, true, _) => Some(EngineKind::Pitwall),
        (EngineChoice::Pitwall, false, _) => None,
        // Automatic: Pitwall served this session and went away: never continue on the other engine silently.
        (_, false, true) => None,
        _ => Some(EngineKind::BuiltIn),
    }
}

/// One engine. Contract: cheap, non-blocking methods (loading happens in the background or on first use), and `llm()` is
/// the port every feature uses; engines never mix within one request.
pub trait AiEngine: Send + Sync {
    fn kind(&self) -> EngineKind;
    fn state(&self) -> LoadState;
    /// What the user calls the model (a file name, or the model Pitwall chose).
    fn model(&self) -> Option<String>;
    fn vision(&self) -> bool;
    /// Gets ready in the background (loads the model); nothing for a server that loads on demand.
    fn preload(&self);
    /// Gives resources back (unloads the model); the engine is not used afterwards.
    fn release(&self);
    fn llm(&self) -> Arc<dyn LlmProvider>;
}

/// The built-in engine: Debrief's worker, the model loaded lazily and unloaded when idle.
pub struct BuiltInEngine {
    pub lazy: Arc<LazyProvider>,
    pub file: String,
}

impl AiEngine for BuiltInEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::BuiltIn
    }
    fn state(&self) -> LoadState {
        self.lazy.state()
    }
    fn model(&self) -> Option<String> {
        Some(self.file.clone())
    }
    fn vision(&self) -> bool {
        self.lazy.vision()
    }
    fn preload(&self) {
        self.lazy.preload();
    }
    fn release(&self) {
        self.lazy.shutdown();
        // Free the worker now (it may hold files a caller is about to delete); a busy one ends with its call.
        self.lazy.unload();
    }
    fn llm(&self) -> Arc<dyn LlmProvider> {
        self.lazy.clone()
    }
}

/// Pitwall as the engine: a server that loads models itself, so it is ready whenever it is usable.
pub struct PitwallEngine {
    pub pitwall: Arc<pitwall::Pitwall>,
    pub priority: pitwall::Priority,
}

/// Pitwall's port when it is not usable at the moment of the call: every request fails with the reason.
struct Unavailable(String);

impl LlmProvider for Unavailable {
    fn id(&self) -> &str {
        "pitwall"
    }
    fn display_name(&self) -> String {
        "Pitwall".into()
    }
    fn context_length(&self) -> usize {
        4096
    }
    fn chat(&self, _: &crate::llm::ChatRequest, _: &mut dyn FnMut(&str)) -> crate::Result<crate::llm::ChatResponse> {
        Err(crate::AiError::Engine(self.0.clone()))
    }
}

impl AiEngine for PitwallEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Pitwall
    }
    fn state(&self) -> LoadState {
        let v = self.pitwall.view();
        if v.usable {
            LoadState::Ready
        } else {
            LoadState::Failed(v.problem.unwrap_or_else(|| "Pitwall is not ready for Debrief.".into()))
        }
    }
    fn model(&self) -> Option<String> {
        self.pitwall.view().model
    }
    fn vision(&self) -> bool {
        self.pitwall.provider(self.priority).is_ok_and(|p| p.vision())
    }
    fn preload(&self) {}
    fn release(&self) {}
    fn llm(&self) -> Arc<dyn LlmProvider> {
        match self.pitwall.provider(self.priority) {
            Ok(p) => Arc::new(p),
            Err(e) => Arc::new(Unavailable(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_parses_leniently_and_round_trips() {
        for c in [EngineChoice::Auto, EngineChoice::Pitwall, EngineChoice::BuiltIn] {
            assert_eq!(EngineChoice::parse(c.as_str()), c);
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(serde_json::from_str::<EngineChoice>(&json).unwrap(), c);
        }
        assert_eq!(EngineChoice::parse("built-in"), EngineChoice::BuiltIn);
        assert_eq!(EngineChoice::parse("cloud"), EngineChoice::Auto);
        assert_eq!(serde_json::from_str::<EngineChoice>("3").unwrap(), EngineChoice::Auto);
    }

    #[test]
    fn selection_never_mixes_engines_silently() {
        use EngineChoice::*;
        use EngineKind as K;
        let s = |choice, usable, served, session| {
            select(SelectInput {
                choice,
                pitwall_usable: usable,
                pitwall_served: served,
                built_in_for_session: session,
            })
        };
        assert_eq!(s(BuiltIn, true, true, false), Some(K::BuiltIn));
        assert_eq!(s(Pitwall, true, false, false), Some(K::Pitwall));
        assert_eq!(s(Pitwall, false, false, false), None);
        assert_eq!(s(Auto, true, false, false), Some(K::Pitwall));
        assert_eq!(s(Auto, false, false, false), Some(K::BuiltIn));
        assert_eq!(s(Auto, false, true, false), None, "Pitwall went away mid-session");
        assert_eq!(s(Auto, true, true, false), Some(K::Pitwall), "it came back");
        assert_eq!(s(Pitwall, false, true, true), Some(K::BuiltIn), "the user chose the fallback");
        assert_eq!(s(Auto, true, true, true), Some(K::BuiltIn));
    }
}
