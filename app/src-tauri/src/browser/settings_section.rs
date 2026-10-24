//! Settings ▸ Web browser: the browser to drive, the search engine for research and "Search the web for this error" (with a
//! user-defined URL template), and the limits of agent mode.

use super::hub::{BrowserHub, BrowserSettings, ProfileSettings};
use super::profile::{DEFAULT_MAX_BYTES, MAX_MAX_BYTES, MIN_MAX_BYTES};
use super::SECTION;
use debrief_ai::browser::gentle::{DEFAULT_MAX_SEARCHES, DEFAULT_PAUSE_MAX, DEFAULT_PAUSE_MIN, MAX_PAUSE, MAX_SEARCHES, MIN_SEARCHES};
use debrief_ai::browser::pace::{
    DEFAULT_ACTIONS_PER_MINUTE, DEFAULT_STEP_LIMIT, MAX_ACTIONS_PER_MINUTE, MAX_STEP_LIMIT, MIN_ACTIONS_PER_MINUTE, MIN_STEP_LIMIT,
};
use debrief_ai::browser::research::{DEFAULT_RESULTS, RESULT_CHOICES};
use debrief_ai::browser::Pacing;
use debrief_ai::browser::{clamp_results, AgentLimits, ResearchSettings, SearchEngine, SearchSettings, SearchTemplate};
use debrief_core::settings::{Configurable, EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::Arc;

pub struct BrowserSection {
    pub hub: Arc<BrowserHub>,
    /// Told which browser the user chose and the kept research profile (none in tests).
    pub renderers: Option<Arc<super::sessions::Renderers>>,
    /// Where the kept research profile lives (Debrief's data folder; none when it cannot be resolved).
    pub profile_dir: Option<std::path::PathBuf>,
}

fn pause() -> FieldKind {
    FieldKind::Duration {
        min_ms: 0,
        max_ms: ms(MAX_PAUSE),
    }
}

fn ms(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
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
            pacing: Pacing::new(
                values.duration("researchPauseMin").unwrap_or(DEFAULT_PAUSE_MIN),
                values.duration("researchPauseMax").unwrap_or(DEFAULT_PAUSE_MAX),
                values
                    .int("researchMaxSearches")
                    .and_then(|v| u8::try_from(v).ok())
                    .unwrap_or(DEFAULT_MAX_SEARCHES),
            ),
        },
        browser_path,
        profile: ProfileSettings {
            keep: values.bool("researchProfile").unwrap_or(false),
            max_bytes: values.bytes("researchProfileMax").unwrap_or(DEFAULT_MAX_BYTES),
        },
    })
}

impl Configurable for BrowserSection {
    fn settings_section(&self) -> SettingsSection {
        let engines = SearchEngine::ALL.iter().map(|e| EnumOption::new(e.id(), e.label())).collect();
        SettingsSection::new(SECTION, "Web browser", SectionOwner::App)
            .describe(
                "The AI's browser: research, Read along and Agent mode drive a real Chromium-based browser (a throwaway profile, or the research profile when Keep a research profile is on; the window shows next to Debrief). The AI sees a page only through research, read-along or agent mode, or when you add one to a chat. Links you open go to your default browser.",
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
            .field(
                SettingField::new("researchPauseMin", "Shortest pause between searches and pages", pause(), ms(DEFAULT_PAUSE_MIN))
                    .describe("Research waits a little before each search and each page it opens, a different time each step between the shortest and the longest pause, so search engines see a human pace and ask less often for a check.")
                    .as_advanced(),
            )
            .field(
                SettingField::new("researchPauseMax", "Longest pause between searches and pages", pause(), ms(DEFAULT_PAUSE_MAX)).as_advanced(),
            )
            .field(
                SettingField::new(
                    "researchMaxSearches",
                    "Searches per question",
                    int(u32::from(MIN_SEARCHES), u32::from(MAX_SEARCHES)),
                    DEFAULT_MAX_SEARCHES,
                )
                .describe("The most results pages one research asks the engine for (the first one and its next pages).")
                .as_advanced(),
            )
            .field(
                SettingField::new("researchProfile", "Keep a research profile (cookies and consent between researches)", FieldKind::Bool, false).describe(
                    "Off: every research browser starts with a throwaway profile. On: research uses a profile of its own in Debrief's data folder (never your browser's), so a consent you gave or a check you passed is remembered and engines ask less often. Clear it below.",
                ),
            )
            .field(
                SettingField::new(
                    "researchProfileMax",
                    "Research profile size limit",
                    FieldKind::Size {
                        min_bytes: MIN_MAX_BYTES,
                        max_bytes: MAX_MAX_BYTES,
                    },
                    DEFAULT_MAX_BYTES,
                )
                .describe("A larger research profile is not used (research uses a throwaway one and says so) until you clear it.")
                .as_advanced(),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        self.hub.set_settings(to_settings(values)?);
        let settings = self.hub.settings();
        if let Some(r) = &self.renderers {
            r.set_configured(settings.browser_path);
            r.set_kept_profile(self.profile_dir.clone(), settings.profile.keep, settings.profile.max_bytes);
        }
        Ok(())
    }
}
