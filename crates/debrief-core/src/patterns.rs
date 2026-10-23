//! Log template mining.
//!
//! Records that already carry a message template (Serilog) are grouped by that
//! template, exactly. Everything else is clustered with the **Drain** algorithm
//! (He et al., "Drain: An Online Log Parsing Approach with Fixed Depth Tree", 2017):
//! variables are masked, then a fixed-depth prefix tree keyed by token count and
//! the first tokens narrows the candidates, and the most similar cluster above a
//! threshold absorbs the line (differing tokens become `<*>`).
//!
//! One deliberate deviation from the paper: a wildcard in an existing template
//! counts as a match when scoring similarity, so templates do not become harder
//! to match as they generalise.

mod trend;
mod values;

pub use trend::{Buckets, Remap, TrendScale, FINE_BUCKETS, TREND_BUCKETS};
pub use values::{Placeholder, PlaceholderValue, COUNTERS, MAX_VALUE_CHARS, TOP_VALUES};

use crate::row::RowRef;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;

/// Placeholder for a variable part of a message.
pub const WILDCARD: &str = "<*>";
/// Messages are truncated to this many tokens before mining.
const MAX_TOKENS: usize = 128;
/// Longest quoted span that is masked as one value.
const MAX_QUOTE_SPAN: usize = 1024;
const MAX_SAMPLES: usize = 5;
/// Most clusters a miner keeps (engine review finding 13); lines that would start another one are
/// counted in one overflow pattern instead.
pub const MAX_CLUSTERS: usize = 10_000;
/// Template of the overflow pattern.
pub const OVERFLOW_TEMPLATE: &str = "(other messages: more distinct patterns than the miner keeps)";

/// Tuning knobs for [`PatternMiner`].
#[derive(Debug, Clone, PartialEq)]
pub struct MinerConfig {
    /// Minimum fraction of matching tokens for a line to join an existing cluster.
    pub similarity_threshold: f64,
    /// Depth of the parse tree; the first `depth - 2` tokens are used for routing.
    pub depth: usize,
    /// Maximum distinct children per tree node before extra tokens share the wildcard child.
    pub max_children: usize,
}

impl Default for MinerConfig {
    fn default() -> Self {
        MinerConfig {
            similarity_threshold: 0.5,
            depth: 4,
            max_children: 100,
        }
    }
}

/// Where a pattern's template came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum TemplateSource {
    /// The record carried its own message template.
    Serilog,
    /// Mined with Drain.
    #[default]
    Mined,
}

/// A group of records sharing one template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pattern {
    pub id: usize,
    pub template: String,
    pub count: usize,
    /// Count per [`Level`] index.
    pub levels: [usize; 6],
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub sample_ids: Vec<u32>,
    pub source: TemplateSource,
    /// Occurrences per time bucket (ADR 0046): [`TREND_BUCKETS`] counts on a scale shared by every pattern of the run;
    /// empty when no event had a timestamp.
    #[serde(default)]
    pub trend: Vec<u32>,
    /// Start of the first trend bucket (epoch ms).
    #[serde(default)]
    pub trend_from: Option<i64>,
    /// Width of a trend bucket in ms (0 when there is no trend).
    #[serde(default)]
    pub trend_bucket_ms: i64,
    /// The values behind the template's `<*>` placeholders (mined templates only), most frequent first.
    #[serde(default)]
    pub placeholders: Vec<Placeholder>,
}

struct Cluster {
    /// Mined template tokens (empty for Serilog clusters).
    tokens: Vec<Box<str>>,
    /// Serilog template (empty for mined clusters).
    serilog: Option<String>,
    count: usize,
    levels: [usize; 6],
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    samples: Vec<u32>,
    trend: Buckets,
    /// The values of the token positions that are placeholders, by position.
    values: Vec<(usize, values::Summary)>,
}

impl Cluster {
    fn new(tokens: Vec<Box<str>>, serilog: Option<String>) -> Self {
        Cluster {
            tokens,
            serilog,
            count: 0,
            levels: [0; 6],
            first_ts: None,
            last_ts: None,
            samples: Vec::new(),
            trend: [0; FINE_BUCKETS],
            values: Vec::new(),
        }
    }

    fn summary_at(&mut self, position: usize) -> Option<&mut values::Summary> {
        self.values.iter_mut().find(|(p, _)| *p == position).map(|(_, s)| s)
    }

    /// Notes `value` for the placeholder at token `position`; `seed` is the literal the position held before it became
    /// a placeholder (with the events that had it), taken only the first time.
    fn record_value(&mut self, position: usize, value: &str, seed: Option<&str>) {
        if self.summary_at(position).is_none() {
            let summary = match seed {
                Some(old) if self.count > 0 => values::Summary::seeded(old, self.count as u64),
                _ => values::Summary::default(),
            };
            self.values.push((position, summary));
        }
        if let Some(s) = self.summary_at(position) {
            s.add(value);
        }
    }

    fn tokens_hold_values(&self, position: usize) -> bool {
        self.values.iter().any(|(p, _)| *p == position)
    }

    /// The placeholders of the template, in the order of its `<*>`s (those that never saw a value are left out).
    fn placeholders(&self) -> Vec<Placeholder> {
        let mut out = Vec::new();
        let mut index = 0;
        for (position, token) in self.tokens.iter().enumerate() {
            let n = token.matches(WILDCARD).count();
            if n > 0 {
                if let Some((_, summary)) = self.values.iter().find(|(p, _)| *p == position) {
                    out.push(summary.finish(index));
                }
            }
            index += n;
        }
        out
    }

    fn observe(&mut self, r: &dyn RowRef) {
        self.count += 1;
        self.levels[r.level().index()] += 1;
        if let Some(ts) = r.ts() {
            self.first_ts = Some(self.first_ts.map_or(ts, |t| t.min(ts)));
            self.last_ts = Some(self.last_ts.map_or(ts, |t| t.max(ts)));
        }
        if self.samples.len() < MAX_SAMPLES {
            self.samples.push(u32::try_from(r.id()).unwrap_or(u32::MAX));
        }
    }
}

#[derive(Default)]
struct Node {
    children: HashMap<Box<str>, Node>,
    clusters: Vec<usize>,
}

/// Incremental template miner.
pub struct PatternMiner {
    config: MinerConfig,
    clusters: Vec<Cluster>,
    by_length: HashMap<usize, Node>,
    serilog: HashMap<String, usize>,
    max_clusters: usize,
    overflow: Option<usize>,
    scale: TrendScale,
}

impl PatternMiner {
    pub fn new(config: MinerConfig) -> Self {
        PatternMiner {
            config,
            clusters: Vec::new(),
            by_length: HashMap::new(),
            serilog: HashMap::new(),
            max_clusters: MAX_CLUSTERS,
            overflow: None,
            scale: TrendScale::default(),
        }
    }

    /// A miner keeping at most `max` clusters (at least 1) besides the overflow pattern.
    pub fn with_limit(config: MinerConfig, max: usize) -> Self {
        PatternMiner {
            max_clusters: max.max(1),
            ..PatternMiner::new(config)
        }
    }

    /// Assigns the event to a cluster (creating it if needed) and returns the cluster id.
    pub fn add<R: RowRef + ?Sized>(&mut self, record: &R) -> usize {
        let id = match record.template().filter(|t| !t.is_empty()) {
            Some(t) => self.add_serilog(t),
            None => self.add_mined(record.message()),
        };
        if let Some(ts) = record.ts() {
            if let Some(remap) = self.scale.fit(ts) {
                for c in &mut self.clusters {
                    c.trend = remap.fold(&c.trend, &self.scale);
                }
            }
            let bucket = self.scale.index(ts);
            let c = &mut self.clusters[id];
            c.trend[bucket] = c.trend[bucket].saturating_add(1);
        }
        self.clusters[id].observe(&record);
        id
    }

    /// Events counted in the overflow pattern because the miner was full.
    pub fn overflowed(&self) -> usize {
        self.overflow.map_or(0, |id| self.clusters[id].count)
    }

    fn full(&self) -> bool {
        self.clusters.len() - usize::from(self.overflow.is_some()) >= self.max_clusters
    }

    fn overflow_cluster(&mut self) -> usize {
        if let Some(id) = self.overflow {
            return id;
        }
        let id = self.clusters.len();
        self.clusters.push(Cluster::new(vec![OVERFLOW_TEMPLATE.into()], None));
        self.overflow = Some(id);
        id
    }

    fn add_serilog(&mut self, template: &str) -> usize {
        if let Some(&id) = self.serilog.get(template) {
            return id;
        }
        if self.full() {
            return self.overflow_cluster();
        }
        let id = self.clusters.len();
        self.clusters.push(Cluster::new(Vec::new(), Some(template.to_string())));
        self.serilog.insert(template.to_string(), id);
        id
    }

    fn add_mined(&mut self, message: &str) -> usize {
        let spans = tokenize_spans(message, MAX_TOKENS);
        let tokens: Vec<&Cow<str>> = spans.iter().map(|t| &t.0).collect();
        let route_depth = self.config.depth.saturating_sub(2).min(tokens.len());
        let max_children = self.config.max_children.max(1);
        let mut node = self.by_length.entry(tokens.len()).or_default();
        for token in tokens.iter().take(route_depth) {
            let key: &str = if token.bytes().any(|b| b.is_ascii_digit()) {
                WILDCARD
            } else {
                token
            };
            // Two lookups keep the borrow checker happy without cloning the key on hits.
            let key = if node.children.contains_key(key) || node.children.len() < max_children {
                key
            } else {
                WILDCARD
            };
            node = node.children.entry(key.into()).or_default();
        }

        let mut best: Option<(usize, f64, usize)> = None;
        for &cid in &node.clusters {
            let (sim, wild) = similarity(&self.clusters[cid].tokens, &tokens);
            let better = match best {
                None => true,
                Some((_, bs, bw)) => sim > bs || (sim == bs && wild > bw),
            };
            if better {
                best = Some((cid, sim, wild));
            }
        }
        if let Some((cid, sim, _)) = best {
            if sim >= self.config.similarity_threshold {
                let cluster = &mut self.clusters[cid];
                for i in 0..cluster.tokens.len().min(tokens.len()) {
                    if *cluster.tokens[i] != **tokens[i] && &*cluster.tokens[i] != WILDCARD {
                        // A literal becomes a placeholder: the events so far all had the old literal.
                        let old = std::mem::replace(&mut cluster.tokens[i], WILDCARD.into());
                        let seed = (!cluster.tokens_hold_values(i)).then_some(old);
                        cluster.record_value(i, value_of(message, &spans[i]), seed.as_deref());
                    } else if cluster.tokens[i].contains(WILDCARD) {
                        cluster.record_value(i, value_of(message, &spans[i]), None);
                    }
                }
                return cid;
            }
        }
        if self.clusters.len() - usize::from(self.overflow.is_some()) >= self.max_clusters {
            return self.overflow_cluster();
        }
        let cid = self.clusters.len();
        let mut cluster = Cluster::new(tokens.iter().map(|t| t.as_ref().into()).collect(), None);
        for (i, span) in spans.iter().enumerate() {
            if tokens[i].contains(WILDCARD) {
                cluster.record_value(i, value_of(message, span), None);
            }
        }
        self.clusters.push(cluster);
        node.clusters.push(cid);
        cid
    }

    /// All clusters so far, most frequent first.
    pub fn clusters(&self) -> Vec<Pattern> {
        let mut out: Vec<Pattern> = self
            .clusters
            .iter()
            .enumerate()
            .map(|(id, c)| Pattern {
                id,
                template: match &c.serilog {
                    Some(t) => t.clone(),
                    None => c.tokens.join(" "),
                },
                count: c.count,
                levels: c.levels,
                first_ts: c.first_ts,
                last_ts: c.last_ts,
                sample_ids: c.samples.clone(),
                source: if c.serilog.is_some() {
                    TemplateSource::Serilog
                } else {
                    TemplateSource::Mined
                },
                trend: if self.scale.is_set() && c.trend.iter().any(|&n| n > 0) {
                    self.scale.display(&c.trend)
                } else {
                    Vec::new()
                },
                trend_from: self.scale.is_set().then(|| self.scale.display_from()),
                trend_bucket_ms: if self.scale.is_set() { self.scale.display_width_ms() } else { 0 },
                placeholders: c.placeholders(),
            })
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
        out
    }
}

/// Mines all `records` and returns their patterns, most frequent first.
pub fn mine<R: RowRef>(records: impl IntoIterator<Item = R>, config: &MinerConfig) -> Vec<Pattern> {
    let mut miner = PatternMiner::new(config.clone());
    for r in records {
        miner.add(&r);
    }
    miner.clusters()
}

/// Fraction of equal tokens (template wildcards always match) and the template's wildcard count.
fn similarity(template: &[Box<str>], tokens: &[&Cow<str>]) -> (f64, usize) {
    if tokens.is_empty() {
        return (1.0, 0);
    }
    let mut same = 0usize;
    let mut wild = 0usize;
    for (a, b) in template.iter().zip(tokens) {
        if &**a == WILDCARD {
            wild += 1;
            same += 1;
        } else if **a == ***b {
            same += 1;
        }
    }
    (same as f64 / tokens.len() as f64, wild)
}

/// What one token contributes as a placeholder value: the text its mask replaced, or the literal token itself.
fn value_of<'a>(message: &'a str, token: &'a Token<'a>) -> &'a str {
    match token.1 {
        Some((a, b)) => message.get(a..b).unwrap_or(""),
        None => &token.0,
    }
}

// ---------------------------------------------------------------------------
// Preprocessing: tokenising and masking variables
// ---------------------------------------------------------------------------

fn is_quote(b: u8) -> bool {
    matches!(b, b'"' | b'\'' | b'`')
}

fn is_trailing(b: u8) -> bool {
    matches!(b, b',' | b';' | b':' | b'.' | b')' | b']' | b'}' | b'!' | b'?')
}

/// Index of the quote that closes the one at `open`, if it plausibly does.
fn find_close(b: &[u8], open: usize) -> Option<usize> {
    let q = b[open];
    let end = b.len().min(open + MAX_QUOTE_SPAN);
    let mut k = open + 1;
    while k < end {
        if b[k] == b'\\' {
            k += 2;
            continue;
        }
        if b[k] == q {
            // An apostrophe inside a word ("don't") is not a closing quote.
            let boundary = k + 1 >= b.len() || b[k + 1].is_ascii_whitespace() || is_trailing(b[k + 1]);
            if q != b'\'' || boundary {
                return Some(k);
            }
        }
        k += 1;
    }
    None
}

/// A masked token and, when a mask replaced something, the byte range of that text in the message.
type Token<'a> = (Cow<'a, str>, Option<(usize, usize)>);

/// Splits a message on whitespace and masks variable tokens (including quoted strings, which may contain spaces).
pub fn tokenize(s: &str, max_tokens: usize) -> Vec<Cow<'_, str>> {
    tokenize_spans(s, max_tokens).into_iter().map(|t| t.0).collect()
}

/// [`tokenize`], with the text each mask replaced (as a byte range of `s`) so placeholder values can be read back.
fn tokenize_spans(s: &str, max_tokens: usize) -> Vec<Token<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() && out.len() < max_tokens {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        let mut quoted = None;
        while j < b.len() && !b[j].is_ascii_whitespace() {
            let opens = j == start || matches!(b[j - 1], b'=' | b'(' | b'[' | b'{' | b',' | b':');
            if opens && is_quote(b[j]) {
                if let Some(close) = find_close(b, j) {
                    quoted = Some((j, close + 1));
                    break;
                }
            }
            j += 1;
        }
        if let Some((open, end)) = quoted {
            let mut k = end;
            while k < b.len() && is_trailing(b[k]) {
                k += 1;
            }
            let inner = if end - open >= 2 { (open + 1, end - 1) } else { (open, end) };
            out.push((Cow::Owned(format!("{}{}{}", &s[start..open], WILDCARD, &s[end..k])), Some(inner)));
            i = k;
        } else {
            let (masked, span) = mask_token(&s[start..j]);
            out.push((masked, span.map(|(a, z)| (start + a, start + z))));
            i = j;
        }
    }
    out
}

/// The masked token and the range of the masked text within `tok`.
fn mask_token(tok: &str) -> (Cow<'_, str>, Option<(usize, usize)>) {
    let is_lead = |c: char| matches!(c, '(' | '[' | '{' | '"' | '\'' | '`');
    let is_trail = |c: char| matches!(c, ')' | ']' | '}' | '"' | '\'' | '`' | ',' | ';' | ':' | '.' | '!' | '?');
    let core = tok.trim_start_matches(is_lead).trim_end_matches(is_trail);
    if core.is_empty() {
        return (Cow::Borrowed(tok), None);
    }
    // `core` is a sub-slice of `tok`, so its offsets are recoverable.
    let lead_len = core.as_ptr() as usize - tok.as_ptr() as usize;
    let trail = &tok[lead_len + core.len()..];
    let lead = &tok[..lead_len];
    if is_variable(core) {
        return (
            Cow::Owned(format!("{lead}{WILDCARD}{trail}")),
            Some((lead_len, lead_len + core.len())),
        );
    }
    if let Some(p) = core.find('=') {
        let value = core[p + 1..].trim_matches(|c: char| is_lead(c) || is_trail(c));
        if p > 0 && is_variable(value) {
            let at = value.as_ptr() as usize - tok.as_ptr() as usize;
            return (
                Cow::Owned(format!("{lead}{}{WILDCARD}{trail}", &core[..=p])),
                Some((at, at + value.len())),
            );
        }
    }
    (Cow::Borrowed(tok), None)
}

fn is_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// True when `c` looks like a value rather than a word: number, hex, GUID, IP, timestamp, path, e-mail.
fn is_variable(c: &str) -> bool {
    if c.is_empty() {
        return false;
    }
    let b = c.as_bytes();
    let has_digit = b.iter().any(u8::is_ascii_digit);
    let body = if matches!(b[0], b'+' | b'-') && b.len() > 1 { &b[1..] } else { b };
    // Numbers, dates, times, IPv4 (with port/CIDR), durations and sizes ("23ms", "5MB", "2024-01-01T10:00:00Z").
    if body[0].is_ascii_digit()
        && body
            .iter()
            .all(|&x| x.is_ascii_alphanumeric() || matches!(x, b'.' | b':' | b',' | b'%' | b'+' | b'-' | b'/'))
    {
        return true;
    }
    // Hashes and ids written in hex (0x-prefixed literals are covered by the number rule).
    if has_digit && b.len() >= 8 && b.iter().all(|&x| is_hex(x)) {
        return true;
    }
    // GUID / UUID.
    if b.len() == 36
        && b.iter()
            .enumerate()
            .all(|(i, &x)| if matches!(i, 8 | 13 | 18 | 23) { x == b'-' } else { is_hex(x) })
    {
        return true;
    }
    // IPv6.
    if b.iter().filter(|&&x| x == b':').count() >= 2 && has_digit && b.iter().all(|&x| is_hex(x) || matches!(x, b':' | b'.' | b'%')) {
        return true;
    }
    // E-mail.
    if let Some(at) = c.find('@') {
        let (l, r) = (&c[..at], &c[at + 1..]);
        if !l.is_empty() && r.contains('.') && !r.ends_with('.') && !r.contains('@') {
            return b
                .iter()
                .all(|&x| x.is_ascii_alphanumeric() || matches!(x, b'.' | b'_' | b'%' | b'+' | b'-' | b'@'));
        }
    }
    // Paths and URLs.
    let seps = b.iter().filter(|&&x| x == b'/' || x == b'\\').count();
    if (b[0] == b'/' && b.len() > 1)
        || c.starts_with("./")
        || c.starts_with("../")
        || c.starts_with("~/")
        || c.starts_with("\\\\")
        || c.contains("://")
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
        || seps >= 2
    {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Comparing two sets of patterns
// ---------------------------------------------------------------------------

/// How a template's share changed between a baseline and a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Change {
    New,
    Gone,
    Spike,
    Drop,
    Same,
}

/// A template's frequency in the baseline versus the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternDelta {
    pub template: String,
    pub baseline_count: usize,
    pub window_count: usize,
    pub change: Change,
    /// Window share divided by baseline share; `None` for [`Change::New`] and [`Change::Gone`].
    pub ratio: Option<f64>,
}

const CHANGE_FACTOR: f64 = 3.0;
const MIN_COUNT: usize = 5;

fn totals(patterns: &[Pattern]) -> (HashMap<&str, usize>, usize) {
    let mut m: HashMap<&str, usize> = HashMap::new();
    let mut total = 0;
    for p in patterns {
        *m.entry(p.template.as_str()).or_default() += p.count;
        total += p.count;
    }
    (m, total)
}

/// Compares template frequencies, normalised by the total of each side. A spike needs at least 5 window
/// occurrences and a drop at least 5 baseline occurrences, so tiny counts do not raise noise.
/// Results are ordered New, Spike, Gone, Drop, Same, then by window count (baseline for Gone/Drop).
pub fn compare(baseline: &[Pattern], window: &[Pattern]) -> Vec<PatternDelta> {
    let (base, base_total) = totals(baseline);
    let (win, win_total) = totals(window);
    let mut keys: Vec<&str> = base.keys().chain(win.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();

    let mut out: Vec<PatternDelta> = keys
        .into_iter()
        .map(|t| {
            let b = base.get(t).copied().unwrap_or(0);
            let w = win.get(t).copied().unwrap_or(0);
            let (change, ratio) = if b == 0 {
                (Change::New, None)
            } else if w == 0 {
                (Change::Gone, None)
            } else {
                let ratio = (w as f64 / win_total as f64) / (b as f64 / base_total as f64);
                let change = if ratio >= CHANGE_FACTOR && w >= MIN_COUNT {
                    Change::Spike
                } else if ratio <= 1.0 / CHANGE_FACTOR && b >= MIN_COUNT {
                    Change::Drop
                } else {
                    Change::Same
                };
                (change, Some(ratio))
            };
            PatternDelta {
                template: t.to_string(),
                baseline_count: b,
                window_count: w,
                change,
                ratio,
            }
        })
        .collect();
    let rank = |c: Change| match c {
        Change::New => 0,
        Change::Spike => 1,
        Change::Gone => 2,
        Change::Drop => 3,
        Change::Same => 4,
    };
    out.sort_by(|a, b| {
        rank(a.change)
            .cmp(&rank(b.change))
            .then((b.window_count.max(b.baseline_count)).cmp(&a.window_count.max(a.baseline_count)))
            .then_with(|| a.template.cmp(&b.template))
    });
    out
}
