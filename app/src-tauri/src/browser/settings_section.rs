//! Settings ▸ Web browser: the browser to drive, the search engine for research and "Search the web for this error" (with a
//! user-defined URL template), and the limits of agent mode.

use super::hub::{BrowserHub, BrowserSettings};
use super::SECTION;
use debrief_ai::browser::pace::{
    DEFAULT_ACTIONS_PER_MINUTE, DEFAULT_STEP_LIMIT, MAX_ACTIONS_PER_MINUTE, MAX_STEP_LIMIT, MIN_ACTIONS_PER_MINUTE, MIN_STEP_LIMIT,
};
use debrief_ai::browser::research::{DEFAULT_RESULTS, RESULT_CHOICES};
use debrief_ai::browser::{clamp_results, AgentLimits, ResearchSettings, SearchEngine, SearchSettings, SearchTemplate};
use debrief_core::settings::{Configurable, EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::Arc;

pub struct BrowserSection {
    pub hub: Arc<BrowserHub>,
    /// Told which browser the user chose (none in tests).
    pub renderers: Option<Arc<super::renderer::Renderers>>,
}

fn int(min: u32, max: u32) -> FieldKind {
    FieldKind::Int {
        min: i64::from(min),
        max: i64::from(max),
        step: 1,
        unit: None,
    }
}

/// Validated values to settings; a custom engine needs a valid template.
pub fn to_settings(values: &ValidValues) -> anyhow::Result<BrowserSettings> {
    let engine = values.text("searchEngine").and_then(SearchEngine::parse).unwrap_or_default();
    let raw = values.text("customSearchUrl").unwrap_or("").trim();
    let custom = if raw.is_empty() {
        None
    } else {
        Some(SearchTemplate::parse(raw).map_err(|e| anyhow::anyhow!("{e}"))?)
    };
    if engine == SearchEngine::Custom && custom.is_none() {
        anyhow::bail!("set the custom search address (with {{q}} where the search text goes) or choose another engine");
    }
    let browser_path = values
        .text("browserPath")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from);
    let num = |k: &str, d: u32| values.int(k).and_then(|v| u32::try_from(v).ok()).unwrap_or(d);
    Ok(BrowserSettings {
        search: SearchSettings { engine, custom },
        agent: AgentLimits {
            step_limit: num("agentStepLimit", DEFAULT_STEP_LIMIT),
            actions_per_minute: num("agentActionsPerMinute", DEFAULT_ACTIONS_PER_MINUTE),
        },
        research: ResearchSettings {
            auto: values.bool("autoResearch").unwrap_or(true),
            max_results: clamp_results(values.text("researchResults").and_then(|t| t.parse().ok())),
        },
        browser_path,
    })
}

impl Configurable for BrowserSection {
    fn settings_section(&self) -> SettingsSection {
        let engines = SearchEngine::ALL.iter().map(|e| EnumOption::new(e.id(), e.label())).collect();
        SettingsSection::new(SECTION, "Web browser", SectionOwner::App)
            .describe(
                "The AI's browser: research, Read along and Agent mode drive a real Chromium-based browser (its own throwaway profile; the window shows next to Debrief). The AI sees a page only through research, read-along or agent mode, or when you add one to a chat. Links you open go to your default browser.",
            )
            .field(
                SettingField::new("searchEngine", "Search engine", FieldKind::Enum { options: engines }, SearchEngine::default().id())
                    .describe("Used by the address box and by Search the web for this error."),
            )
            .field(
                SettingField::new(
                    "customSearchUrl",
                    "Custom search address",
                    FieldKind::Text {
                        max_len: debrief_ai::browser::engines::MAX_TEMPLATE_CHARS,
                        pattern: None,
                    },
                    "",
                )
                .describe("For the Custom engine: an https address with {q} where the search text goes, for example https://search.example.org/?q={q}."),
            )
            .field(
                SettingField::new("browserPath", "Browser to use", FieldKind::Text { max_len: 1024, pattern: None }, "").describe(
                    "Research and the chat's browser modes drive a Chromium-based browser (Edge, Chrome or Chromium). Leave empty to use the downloaded Chromium or the one found on this computer.",
                ),
            )
            .field(
                SettingField::new("agentStepLimit", "Agent steps per task", int(MIN_STEP_LIMIT, MAX_STEP_LIMIT), DEFAULT_STEP_LIMIT)
                    .describe("Agent mode stops after this many steps and asks whether to continue."),
            )
            .field(
                SettingField::new(
                    "agentActionsPerMinute",
                    "Agent actions per minute",
                    int(MIN_ACTIONS_PER_MINUTE, MAX_ACTIONS_PER_MINUTE),
                    DEFAULT_ACTIONS_PER_MINUTE,
                )
                .describe("Agent mode browses at a human pace: at most this many actions in a minute."),
            )
            .field(
                SettingField::new("autoResearch", "Let the AI research the web automatically", FieldKind::Bool, true).describe(
                    "The AI searches and reads the top results by itself (read-only) when a question needs outside information. Off: it offers a Research this button instead.",
                ),
            )
            .field(
                SettingField::new(
                    "researchResults",
                    "Results to read per search",
                    FieldKind::Enum {
                        options: RESULT_CHOICES.iter().map(|n| EnumOption::new(n.to_string(), n.to_string())).collect(),
                    },
                    DEFAULT_RESULTS.to_string(),
                )
                .describe("Research reads this many results the AI picked, over the engine's next results pages when needed, then answers."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        self.hub.set_settings(to_settings(values)?);
        if let Some(r) = &self.renderers {
            r.set_configured(self.hub.settings().browser_path);
        }
        Ok(())
    }
}
