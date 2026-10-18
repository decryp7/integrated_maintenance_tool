//! What the follower reads: validated channel names, levels, the backfill
//! window and the user's XPath filter, combined into one Event Log XPath.
//!
//! Everything here is parsed once from untrusted input (IPC JSON) into types
//! and is pure: the Windows Event Log re-validates the final query anyway.

use anyhow::{bail, Result};
use serde::Serialize;
use std::fmt;

/// The classic channels every Windows installation has.
pub const STANDARD_CHANNELS: [&str; 5] = ["System", "Application", "Security", "Setup", "ForwardedEvents"];
/// Longest accepted channel name.
pub const MAX_CHANNEL_LEN: usize = 256;
/// Longest accepted user XPath filter.
pub const MAX_XPATH_LEN: usize = 2048;
/// Most channels one follower reads (one reader thread each).
pub const MAX_CHANNELS: usize = 32;

/// A channel the follower may read: a standard channel or a
/// `Microsoft-Windows-*/Operational` channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Channel(String);

impl Channel {
    pub fn parse(name: &str) -> Result<Channel> {
        let name = name.trim();
        if let Some(std) = STANDARD_CHANNELS.iter().find(|c| c.eq_ignore_ascii_case(name)) {
            return Ok(Channel((*std).to_string()));
        }
        if name.chars().count() > MAX_CHANNEL_LEN {
            bail!("the channel name is longer than {MAX_CHANNEL_LEN} characters");
        }
        if is_operational(name) && name.chars().all(|c| c.is_ascii_alphanumeric() || " -_.%/".contains(c)) {
            return Ok(Channel(name.to_string()));
        }
        bail!(
            "\"{}\" is not a channel the follower reads: choose {} or a Microsoft-Windows-*/Operational channel",
            name.chars().take(80).collect::<String>(),
            STANDARD_CHANNELS.join(", ")
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reading it needs administrator rights.
    pub fn needs_admin(&self) -> bool {
        self.0 == "Security"
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `Microsoft-Windows-<something>/Operational` with exactly one `/`.
fn is_operational(name: &str) -> bool {
    name.strip_prefix("Microsoft-Windows-")
        .and_then(|rest| rest.strip_suffix("/Operational"))
        .is_some_and(|middle| !middle.is_empty() && !middle.contains('/'))
}

/// The channels worth offering from everything `EvtOpenChannelEnum` lists:
/// the standard ones first, then the operational channels, sorted.
pub fn offered_channels<I: IntoIterator<Item = String>>(all: I) -> Vec<Channel> {
    let mut operational: Vec<Channel> = all
        .into_iter()
        .filter(|n| is_operational(n))
        .filter_map(|n| Channel::parse(&n).ok())
        .collect();
    operational.sort_by_key(|c| c.0.to_ascii_lowercase());
    operational.dedup();
    STANDARD_CHANNELS
        .iter()
        .map(|c| Channel((*c).to_string()))
        .chain(operational)
        .collect()
}

/// An Event Log level as users pick it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EventLevel {
    Critical,
    Error,
    Warning,
    Information,
    Verbose,
}

impl EventLevel {
    pub const ALL: [EventLevel; 5] = [
        EventLevel::Critical,
        EventLevel::Error,
        EventLevel::Warning,
        EventLevel::Information,
        EventLevel::Verbose,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EventLevel::Critical => "critical",
            EventLevel::Error => "error",
            EventLevel::Warning => "warning",
            EventLevel::Information => "information",
            EventLevel::Verbose => "verbose",
        }
    }

    pub fn parse(name: &str) -> Result<EventLevel> {
        EventLevel::ALL
            .into_iter()
            .find(|l| l.name().eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "\"{}\" is not a level: use {}",
                    name.chars().take(40).collect::<String>(),
                    EventLevel::ALL.map(EventLevel::name).join(", ")
                )
            })
    }

    /// The `System/Level` values: Information covers 4 and 0 ("log always",
    /// e.g. security audits).
    pub fn system_levels(self) -> &'static [u8] {
        match self {
            EventLevel::Critical => &[1],
            EventLevel::Error => &[2],
            EventLevel::Warning => &[3],
            EventLevel::Information => &[0, 4],
            EventLevel::Verbose => &[5],
        }
    }
}

/// The levels to read; empty or all five means every event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelSet(Vec<EventLevel>);

impl LevelSet {
    pub fn new<I: IntoIterator<Item = EventLevel>>(levels: I) -> LevelSet {
        let mut v: Vec<EventLevel> = levels.into_iter().collect();
        v.sort();
        v.dedup();
        LevelSet(v)
    }

    pub fn is_all(&self) -> bool {
        self.0.is_empty() || self.0.len() == EventLevel::ALL.len()
    }

    pub fn levels(&self) -> &[EventLevel] {
        &self.0
    }

    /// `System[(Level=1 or Level=2)]`, or `None` for every level.
    pub fn predicate(&self) -> Option<String> {
        if self.is_all() {
            return None;
        }
        let mut numbers: Vec<u8> = self.0.iter().flat_map(|l| l.system_levels().iter().copied()).collect();
        numbers.sort_unstable();
        let terms: Vec<String> = numbers.iter().map(|n| format!("Level={n}")).collect();
        Some(format!("System[({})]", terms.join(" or ")))
    }
}

/// How far back the first run reads before following live events.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum Backfill {
    /// Only events written from now on.
    #[default]
    None,
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
}

impl Backfill {
    pub const NAMES: [&'static str; 4] = ["none", "1h", "24h", "7d"];

    pub fn parse(name: &str) -> Result<Backfill> {
        Ok(match name.trim() {
            "" | "none" => Backfill::None,
            "1h" => Backfill::Hour,
            "24h" => Backfill::Day,
            "7d" => Backfill::Week,
            other => bail!(
                "\"{}\" is not a backfill window: use {}",
                other.chars().take(40).collect::<String>(),
                Backfill::NAMES.join(", ")
            ),
        })
    }

    /// The window in milliseconds, `None` for no backfill.
    pub fn millis(self) -> Option<u64> {
        match self {
            Backfill::None => None,
            Backfill::Hour => Some(3_600_000),
            Backfill::Day => Some(86_400_000),
            Backfill::Week => Some(604_800_000),
        }
    }

    /// `System[TimeCreated[timediff(@SystemTime) <= N]]`.
    pub fn predicate(self) -> Option<String> {
        self.millis()
            .map(|ms| format!("System[TimeCreated[timediff(@SystemTime) <= {ms}]]"))
    }
}

/// A user XPath filter, reduced to the predicate inside `*[...]`. Accepts
/// what Event Viewer's XML tab shows inside `<Select>` (`*[System[...]]`)
/// or a bare predicate (`EventData[Data[@Name='x']='y']`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XPathFilter(String);

impl XPathFilter {
    /// `None` for an empty filter.
    pub fn parse(text: &str) -> Result<Option<XPathFilter>> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        let n = t.chars().count();
        if n > MAX_XPATH_LEN {
            bail!("the XPath filter is longer than {MAX_XPATH_LEN} characters ({n})");
        }
        if t.starts_with('<') {
            bail!("paste the XPath inside <Select>...</Select>, not the whole QueryList XML");
        }
        if let Some(c) = t.chars().find(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')) {
            bail!("the XPath filter contains a control character (U+{:04X})", c as u32);
        }
        check_balanced(t)?;
        let inner = strip_select(t).trim();
        if inner.is_empty() || inner == "*" {
            return Ok(None);
        }
        Ok(Some(XPathFilter(inner.to_string())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `*[P]` or `Event[P]` (outer brackets closing at the end) becomes `P`.
fn strip_select(t: &str) -> &str {
    let body = t.strip_prefix("*[").or_else(|| t.strip_prefix("Event["));
    let Some(body) = body.and_then(|b| b.strip_suffix(']')) else {
        return t;
    };
    // Only when the first `[` closes at the very end (not `*[a] and *[b]`).
    if check_balanced(body).is_ok() {
        body
    } else {
        t
    }
}

/// Brackets and parentheses nest and quotes close.
fn check_balanced(t: &str) -> Result<()> {
    let mut stack: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    for c in t.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '[' | '(' => stack.push(c),
            ']' | ')' => {
                let open = if c == ']' { '[' } else { '(' };
                if stack.pop() != Some(open) {
                    bail!("the XPath filter has an unmatched '{c}'");
                }
            }
            _ => {}
        }
        if stack.len() > 64 {
            bail!("the XPath filter nests too deeply");
        }
    }
    if let Some(q) = quote {
        bail!("the XPath filter has an unclosed {q} quote");
    }
    if let Some(open) = stack.pop() {
        bail!("the XPath filter has an unclosed '{open}'");
    }
    Ok(())
}

/// The Event Log XPath for levels, an optional backfill window and the user
/// filter: `*` or `*[a and b and (user)]`.
pub fn build_xpath(levels: &LevelSet, backfill: Backfill, filter: Option<&XPathFilter>) -> String {
    let parts: Vec<String> = [
        levels.predicate(),
        backfill.predicate(),
        filter.map(|f| format!("({})", f.as_str())),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        "*".into()
    } else {
        format!("*[{}]", parts.join(" and "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_standard_or_operational() {
        assert_eq!(Channel::parse(" system ").unwrap().as_str(), "System");
        assert!(Channel::parse("Security").unwrap().needs_admin());
        assert!(!Channel::parse("Setup").unwrap().needs_admin());
        let op = Channel::parse("Microsoft-Windows-TaskScheduler/Operational").unwrap();
        assert_eq!(op.to_string(), "Microsoft-Windows-TaskScheduler/Operational");
        for bad in [
            "",
            "Microsoft-Windows-/Operational",
            "Microsoft-Windows-A/B/Operational",
            "Microsoft-Windows-X/Admin",
            "Microsoft-Windows-X'/Operational",
            "C:\\Windows\\System32\\winevt\\Logs\\System.evtx",
            "Windows PowerShell",
        ] {
            let e = Channel::parse(bad).unwrap_err().to_string();
            assert!(e.contains("not a channel the follower reads"), "{bad}: {e}");
        }
        let long = format!("Microsoft-Windows-{}/Operational", "a".repeat(300));
        assert!(Channel::parse(&long).unwrap_err().to_string().contains("longer"));
    }

    #[test]
    fn offers_standard_then_sorted_operational_channels() {
        let all = [
            "Microsoft-Windows-Zeta/Operational",
            "Application",
            "Microsoft-Windows-alpha/Operational",
            "Microsoft-Windows-Zeta/Operational",
            "Microsoft-Windows-Kernel-PnP/Configuration",
            "HardwareEvents",
        ]
        .map(String::from);
        let names: Vec<String> = offered_channels(all).into_iter().map(|c| c.0).collect();
        assert_eq!(
            names,
            [
                "System",
                "Application",
                "Security",
                "Setup",
                "ForwardedEvents",
                "Microsoft-Windows-alpha/Operational",
                "Microsoft-Windows-Zeta/Operational"
            ]
        );
    }

    #[test]
    fn levels_map_to_system_levels() {
        assert_eq!(EventLevel::parse(" Warning ").unwrap(), EventLevel::Warning);
        assert!(EventLevel::parse("loud").unwrap_err().to_string().contains("critical, error"));
        for l in EventLevel::ALL {
            assert_eq!(EventLevel::parse(l.name()).unwrap(), l);
        }
        assert_eq!(LevelSet::default().predicate(), None);
        assert_eq!(LevelSet::new(EventLevel::ALL).predicate(), None);
        let set = LevelSet::new([EventLevel::Information, EventLevel::Critical, EventLevel::Critical]);
        assert_eq!(set.levels(), [EventLevel::Critical, EventLevel::Information]);
        assert_eq!(set.predicate().unwrap(), "System[(Level=0 or Level=1 or Level=4)]");
        assert_eq!(LevelSet::new([EventLevel::Verbose]).predicate().unwrap(), "System[(Level=5)]");
    }

    #[test]
    fn backfill_windows() {
        assert_eq!(Backfill::parse("").unwrap(), Backfill::None);
        assert_eq!(Backfill::parse("none").unwrap().predicate(), None);
        assert_eq!(Backfill::parse("1h").unwrap().millis(), Some(3_600_000));
        assert_eq!(Backfill::parse("24h").unwrap(), Backfill::Day);
        assert_eq!(
            Backfill::parse("7d").unwrap().predicate().unwrap(),
            "System[TimeCreated[timediff(@SystemTime) <= 604800000]]"
        );
        assert!(Backfill::parse("1y").unwrap_err().to_string().contains("none, 1h, 24h, 7d"));
    }

    #[test]
    fn xpath_filters_are_validated_and_reduced() {
        assert_eq!(XPathFilter::parse("  ").unwrap(), None);
        assert_eq!(XPathFilter::parse("*").unwrap(), None);
        assert_eq!(XPathFilter::parse("*[]").unwrap(), None);
        let f = XPathFilter::parse("*[System[Provider[@Name='Service Control Manager']]]")
            .unwrap()
            .unwrap();
        assert_eq!(f.as_str(), "System[Provider[@Name='Service Control Manager']]");
        let f = XPathFilter::parse("Event[System[EventID=7036]]").unwrap().unwrap();
        assert_eq!(f.as_str(), "System[EventID=7036]");
        let both = "*[System[EventID=1]] or *[System[EventID=2]]";
        assert_eq!(XPathFilter::parse(both).unwrap().unwrap().as_str(), both);
        let bare = "EventData[Data[@Name='TargetUserName']='a]b']";
        assert_eq!(XPathFilter::parse(bare).unwrap().unwrap().as_str(), bare);
        for (bad, why) in [
            ("System[EventID=1", "unclosed '['"),
            ("System[EventID=1]]", "unmatched ']'"),
            ("(System[EventID=1]", "unclosed '('"),
            ("System[(EventID=1])", "unmatched ']'"),
            ("System[Provider[@Name='x]]", "unclosed ' quote"),
            ("<QueryList><Query/></QueryList>", "not the whole QueryList"),
            ("System[EventID=1\u{0}]", "control character"),
        ] {
            let e = XPathFilter::parse(bad).unwrap_err().to_string();
            assert!(e.contains(why), "{bad}: {e}");
        }
        assert!(XPathFilter::parse(&"[".repeat(70)).unwrap_err().to_string().contains("deeply"));
        let long = format!("System[EventID={}]", "1".repeat(MAX_XPATH_LEN));
        assert!(XPathFilter::parse(&long).unwrap_err().to_string().contains("longer"));
    }

    #[test]
    fn builds_the_combined_xpath() {
        assert_eq!(build_xpath(&LevelSet::default(), Backfill::None, None), "*");
        let f = XPathFilter::parse("System[EventID=7036]").unwrap();
        assert_eq!(
            build_xpath(&LevelSet::default(), Backfill::None, f.as_ref()),
            "*[(System[EventID=7036])]"
        );
        assert_eq!(
            build_xpath(&LevelSet::new([EventLevel::Error]), Backfill::Hour, f.as_ref()),
            "*[System[(Level=2)] and System[TimeCreated[timediff(@SystemTime) <= 3600000]] and (System[EventID=7036])]"
        );
    }
}
