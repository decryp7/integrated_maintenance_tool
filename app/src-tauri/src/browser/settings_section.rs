//! Settings ▸ Web browser: the search engine of the address box and "Search the web for this error" (with a
//! user-defined URL template), and the limits of agent mode.

use super::hub::{BrowserHub, BrowserSettings};
use super::SECTION;
use imt_ai::browser::pace::{
    DEFAULT_ACTIONS_PER_MINUTE, DEFAULT_STEP_LIMIT, MAX_ACTIONS_PER_MINUTE, MAX_STEP_LIMIT, MIN_ACTIONS_PER_MINUTE, MIN_STEP_LIMIT,
};
use imt_ai::browser::research::{DEFAULT_RESULTS, MAX_RESULTS, MIN_RESULTS};
use imt_ai::browser::{clamp_results, AgentLimits, ResearchSettings, SearchEngine, SearchSettings, SearchTemplate};
use imt_core::settings::{Configurable, EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::Arc;

pub struct BrowserSection {
    pub hub: Arc<BrowserHub>,
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
    let num = |k: &str, d: u32| values.int(k).and_then(|v| u32::try_from(v).ok()).unwrap_or(d);
    Ok(BrowserSettings {
        search: SearchSettings { engine, custom },
        agent: AgentLimits {
            step_limit: num("agentStepLimit", DEFAULT_STEP_LIMIT),
            actions_per_minute: num("agentActionsPerMinute", DEFAULT_ACTIONS_PER_MINUTE),
        },
        research: ResearchSettings {
            auto: values.bool("autoResearch").unwrap_or(true),
            max_results: clamp_results(values.int("researchResults")),
        },
    })
}

impl Configurable for BrowserSection {
    fn settings_section(&self) -> SettingsSection {
        let engines = SearchEngine::ALL.iter().map(|e| EnumOption::new(e.id(), e.label())).collect();
        SettingsSection::new(SECTION, "Web browser", SectionOwner::App)
            .describe(
                "IMT's internal browser: you browse; the AI sees a page only when you add it to a chat, turn on read-along or agent mode for a chat. Its cookies and history are kept apart from IMT.",
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
                        max_len: imt_ai::browser::engines::MAX_TEMPLATE_CHARS,
                        pattern: None,
                    },
                    "",
                )
                .describe("For the Custom engine: an https address with {q} where the search text goes, for example https://search.example.org/?q={q}."),
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
                    int(u32::from(MIN_RESULTS), u32::from(MAX_RESULTS)),
                    u32::from(DEFAULT_RESULTS),
                )
                .describe("How many result pages research opens and reads (a request can ask for another number, up to 10)."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        self.hub.set_settings(to_settings(values)?);
        Ok(())
    }
}
