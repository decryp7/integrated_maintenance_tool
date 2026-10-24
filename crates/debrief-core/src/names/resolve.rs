//! Resolving the user's words to entities the way a person would: every match says how it was made and why, context
//! ranks candidates, and a true tie becomes a short question instead of a guess.
//!
//! The ways a group matches ([`How`]), best first: the user's own alias (they said so), the same letters as a
//! spelling in the logs (exact), the same words written differently (segmented: `graphic builder` is
//! `graphicbuilder.exe`), the words as parts of the name, words that start the name's words, an AI alias, and a typo.
//! The score is the way's base, a little for how frequent the group is, more for the context ([`super::context`]) and
//! for closeness to the user's anchors. Two different groups matching the same way with scores closer than
//! [`TIE_GAP`] are a true tie: [`Resolution::clarify`] holds the question.

use super::book::{NameBook, Prepared};
use super::context::{relevance, Cooccurrence, SearchContext, MAX_ANCHORS};
use super::group::{NameGroup, Spelling};
use super::normalise::normalise;
use super::segment::Vocabulary;
use crate::catalogue::Scope;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Scores closer than this between different groups of the same way are a tie.
pub const TIE_GAP: f64 = 0.04;
/// Most matches returned.
pub const MAX_MATCHES: usize = 12;
/// Longest text resolved (characters).
pub const MAX_TEXT_CHARS: usize = 256;
/// Most options in a clarifying question.
pub const MAX_OPTIONS: usize = 3;
/// Words people put in front of a name that are not part of it.
const FILLER: &[&str] = &["the", "a", "an", "my", "our", "that", "this", "please"];
const POPULARITY_WEIGHT: f64 = 0.02;
const CONTEXT_WEIGHT: f64 = 0.15;
const COOCCURRENCE_WEIGHT: f64 = 0.1;

/// Letters and digits only, lower-cased.
pub fn squash(s: &str) -> String {
    crate::catalogue::squash(s)
}

/// How a group matched the words, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum How {
    /// The user said so (an accepted match, an alias or a rename).
    UserAlias,
    /// Same letters as a spelling in the logs.
    Exact,
    /// The same words written differently (`graphic builder`, `graphicbuilder.exe`).
    Segmented,
    /// The words are consecutive parts of the name.
    Parts,
    /// Every word starts a word of the name.
    Prefix,
    /// An alias a local model suggested.
    Alias,
    /// Close in spelling.
    Typo,
}

impl How {
    fn base(self) -> f64 {
        match self {
            How::UserAlias => 1.3,
            How::Exact => 1.0,
            How::Segmented => 0.95,
            How::Parts => 0.85,
            How::Alias => 0.8,
            How::Prefix => 0.7,
            How::Typo => 0.55,
        }
    }

    /// The id shown to the assistant and the UI.
    pub fn id(self) -> &'static str {
        match self {
            How::UserAlias => "userAlias",
            How::Exact => "exact",
            How::Segmented => "segmented",
            How::Parts => "parts",
            How::Prefix => "prefix",
            How::Alias => "alias",
            How::Typo => "typo",
        }
    }
}

/// The Settings switches of the name matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameSettings {
    /// Use AI aliases (Settings: off removes their effect; the cached aliases stay).
    pub ai_aliases: bool,
    /// Rank by the user's context.
    pub context: bool,
}

impl Default for NameSettings {
    fn default() -> Self {
        NameSettings {
            ai_aliases: true,
            context: true,
        }
    }
}

/// A group that matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameMatch {
    /// The group's id, for feedback ([`NameBook::accept`]).
    pub id: String,
    pub scope: Scope,
    /// The name as people say it (`Graphic Builder`).
    pub display: String,
    /// The most frequent spelling in the logs.
    pub canonical: String,
    /// Every spelling with its own count.
    pub spellings: Vec<Spelling>,
    pub count: u64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub seen_in: Vec<String>,
    pub how: How,
    pub score: f64,
    /// One short phrase: how it matched, then what in the context supports it.
    pub why: String,
}

/// A question to ask instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clarification {
    pub question: String,
    /// The group ids that tie, best first.
    pub options: Vec<String>,
}

/// The answer of [`Resolver::resolve`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub matches: Vec<NameMatch>,
    pub clarify: Option<Clarification>,
}

/// Resolves words against a [`NameBook`].
pub struct Resolver<'a> {
    pub book: &'a NameBook,
    pub vocab: &'a Vocabulary,
    pub ctx: &'a SearchContext,
    pub settings: NameSettings,
}

impl Resolver<'_> {
    /// The groups the words refer to, best first, at most `limit` (and [`MAX_MATCHES`]). `cooc` are the events'
    /// names around the user's anchors, when they were sampled.
    pub fn resolve(&self, text: &str, limit: usize, cooc: Option<&Cooccurrence>) -> Resolution {
        let text: String = text.trim().chars().take(MAX_TEXT_CHARS).collect();
        let q = squash(&text);
        if q.is_empty() {
            return Resolution::default();
        }
        let qn = normalise(&text, self.vocab, &HashSet::new());
        let qtokens = self.tidy(qn.tokens);
        let qjoined = qtokens.concat();
        let max_count = self.book.groups().iter().map(|g| g.count).max().unwrap_or(1).max(1);
        let mut found: Vec<NameMatch> = Vec::new();
        let rejected = self.book.has_rejections();
        for (g, p) in self.book.groups().iter().zip(self.book.prepared()) {
            let id = p.id.clone();
            if rejected && self.book.is_rejected(&text, &id) {
                continue;
            }
            let Some((how, detail)) = self.how(g, p, &text, &q, &qtokens, &qjoined) else {
                continue;
            };
            let mut score = how.base() + POPULARITY_WEIGHT * ((1.0 + g.count as f64).ln() / (1.0 + max_count as f64).ln());
            let mut reasons: Vec<String> = vec![detail];
            if how != How::UserAlias && self.settings.context {
                let (r, why) = relevance(g, self.ctx);
                score += CONTEXT_WEIGHT * r;
                reasons.extend(why.into_iter().map(String::from));
                if let Some(c) = cooc {
                    if let Some((s, anchor)) = c.closeness(g, &self.ctx.anchors[..self.ctx.anchors.len().min(MAX_ANCHORS)]) {
                        score += COOCCURRENCE_WEIGHT * s;
                        reasons.push(format!("appears in the same events as {anchor}"));
                    }
                }
            }
            found.push(NameMatch {
                id,
                scope: g.scope.clone(),
                display: g.display.clone(),
                canonical: g.canonical().to_string(),
                spellings: g.spellings.clone(),
                count: g.count,
                first_ts: g.first_ts,
                last_ts: g.last_ts,
                seen_in: g.seen_in.clone(),
                how,
                score,
                why: reasons.join("; "),
            });
        }
        found.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(b.count.cmp(&a.count))
                .then_with(|| a.id.cmp(&b.id))
        });
        // Typos only when nothing matches better.
        if found.iter().any(|m| m.how != How::Typo) {
            found.retain(|m| m.how != How::Typo);
        }
        found.truncate(limit.clamp(1, MAX_MATCHES));
        let clarify = clarification(&found, &text);
        Resolution { matches: found, clarify }
    }

    /// The query's words without filler (`the`, `my`) and with plurals made singular when the singular is a known word.
    fn tidy(&self, tokens: Vec<String>) -> Vec<String> {
        let kept: Vec<String> = tokens.iter().filter(|t| !FILLER.contains(&t.as_str())).cloned().collect();
        let tokens = if kept.is_empty() { tokens } else { kept };
        tokens
            .into_iter()
            .map(|t| match t.strip_suffix('s') {
                Some(one) if one.chars().count() >= 3 && !one.ends_with('s') && self.vocab.knows(one) && !self.vocab.knows(&t) => {
                    one.to_string()
                }
                _ => t,
            })
            .collect()
    }

    /// How `g` matches, with the phrase saying so.
    fn how(&self, g: &NameGroup, p: &Prepared, text: &str, q: &str, qtokens: &[String], qjoined: &str) -> Option<(How, String)> {
        let id = p.id.as_str();
        if self.book.has_user_terms() && self.book.user_terms(id).iter().any(|(t, _)| squash(t) == q) {
            return Some((How::UserAlias, "you told Debrief this name".into()));
        }
        if p.spellings.iter().any(|s| s == q) {
            return Some((How::Exact, "the name as written in the logs".into()));
        }
        let gjoined = p.joined.as_str();
        if !qjoined.is_empty() && gjoined == qjoined {
            let phrase = if qtokens.len() != p.tokens.len() {
                "same name written as one word"
            } else {
                "same name spelled differently (case, separators, version)"
            };
            return Some((How::Segmented, phrase.into()));
        }
        let ktokens: Vec<&str> = p.tokens.iter().map(String::as_str).collect();
        if !qtokens.is_empty()
            && qtokens.len() < ktokens.len()
            && ktokens.windows(qtokens.len()).any(|w| w.iter().zip(qtokens).all(|(a, b)| a == b))
        {
            return Some((How::Parts, "your words are parts of this name".into()));
        }
        if crate::words::phrase_in(text, &g.key) && g.key.len() > text.trim().len() {
            return Some((How::Parts, "your words are parts of this name".into()));
        }
        if !qtokens.is_empty()
            && qtokens
                .iter()
                .all(|t| t.chars().count() >= 2 && ktokens.iter().any(|k| k.starts_with(t.as_str())))
        {
            return Some((How::Prefix, "your words start the words of this name".into()));
        }
        if self.settings.ai_aliases {
            if let Some(e) = self.book.ai(id) {
                let hit = std::iter::once(&e.human)
                    .chain(&e.aliases)
                    .find(|a| !a.is_empty() && squash(a) == q);
                if let Some(a) = hit {
                    return Some((
                        How::Alias,
                        format!("AI alias: '{a}' (suggested by a local model; you can remove it)"),
                    ));
                }
            }
        }
        // A typo has a word nobody knows; "grape builder" is two real words, so it is another thing.
        let misspelt = qtokens.iter().any(|t| !self.vocab.knows(t));
        let d = if qtokens.len() == ktokens.len() && ktokens.len() > 1 {
            // Word by word: each word may be a letter or two off, but a different word is another thing.
            qtokens
                .iter()
                .zip(&ktokens)
                .map(|(a, b)| edit_distance_capped(a, b, (b.chars().count() / 3).max(1)))
                .sum::<Option<usize>>()
        } else {
            edit_distance_capped(q, gjoined, allowed_typos(gjoined.chars().count()))
        };
        if let Some(d) = d.filter(|_| q.chars().count() >= 4 && misspelt) {
            return Some((
                How::Typo,
                format!("close spelling ({d} letter{} off)", if d == 1 { "" } else { "s" }),
            ));
        }
        if self.settings.ai_aliases && q.chars().count() >= 4 && misspelt {
            if let Some(e) = self.book.ai(id) {
                let hit = e.aliases.iter().find(|a| {
                    let s = squash(a);
                    edit_distance_capped(q, &s, allowed_typos(s.chars().count())).is_some()
                });
                if let Some(a) = hit {
                    return Some((How::Typo, format!("close spelling of the AI alias '{a}'")));
                }
            }
        }
        None
    }
}

fn allowed_typos(len: usize) -> usize {
    (len / 4).max(1)
}

/// Edit distance (insert, delete, substitute, swap of neighbours) when it is at most `cap`, else none. Inputs are cut
/// at 64 characters.
pub fn edit_distance_capped(a: &str, b: &str, cap: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().take(64).collect();
    let b: Vec<char> = b.chars().take(64).collect();
    if a.len().abs_diff(b.len()) > cap {
        return None;
    }
    let mut prev2: Vec<usize> = vec![0; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            let mut best = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
            if i > 0 && j > 0 && *ca == b[j - 1] && a[i - 1] == *cb {
                best = best.min(prev2[j - 1] + 1);
            }
            cur.push(best);
        }
        prev2 = prev;
        prev = cur;
    }
    let d = prev[b.len()];
    (d <= cap).then_some(d)
}

/// The question for a true tie at the top of `found`, if there is one.
fn clarification(found: &[NameMatch], text: &str) -> Option<Clarification> {
    let top = found.first()?;
    if top.how == How::UserAlias {
        return None;
    }
    let tied: Vec<&NameMatch> = found
        .iter()
        .take_while(|m| m.how == top.how && top.score - m.score < TIE_GAP)
        .take(MAX_OPTIONS)
        .collect();
    if tied.len() < 2 {
        return None;
    }
    let names: Vec<String> = tied.iter().map(|m| format!("{} ({})", m.display, kind_word(&m.scope))).collect();
    // At least two options: "A or B", "A, B or C".
    let (last, init) = names.split_last()?;
    let list = format!("{} or {last}", init.join(", "));
    Some(Clarification {
        question: format!("By '{}', do you mean {list}?", text.trim()),
        options: tied.iter().map(|m| m.id.clone()).collect(),
    })
}

/// What a scope is, in a word a person says.
pub fn kind_word(scope: &Scope) -> String {
    use crate::entities::EntityKind::*;
    match scope {
        Scope::Entity(Executable) => "process".into(),
        Scope::Entity(Library) => "library".into(),
        Scope::Entity(File) => "file".into(),
        Scope::Entity(Exception) => "exception".into(),
        Scope::Entity(k) => k.id().to_string(),
        Scope::Field(f) => f.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::EntityKind;
    use crate::names::book::AiEntry;

    fn grp(key: &str, spell: &[(&str, u64)], seen: &str, ts: (i64, i64)) -> NameGroup {
        NameGroup {
            scope: Scope::Entity(EntityKind::Executable),
            key: key.into(),
            display: key
                .split(' ')
                .map(|w| w[..1].to_uppercase() + &w[1..])
                .collect::<Vec<_>>()
                .join(" "),
            spellings: spell
                .iter()
                .map(|(n, c)| Spelling {
                    name: n.to_string(),
                    count: *c,
                })
                .collect(),
            count: spell.iter().map(|s| s.1).sum(),
            first_ts: Some(ts.0),
            last_ts: Some(ts.1),
            seen_in: vec![seen.into()],
        }
    }

    fn book() -> NameBook {
        let mut b = NameBook::default();
        b.rebuild(vec![
            grp(
                "graphic builder",
                &[
                    ("graphicbuilder.exe", 12),
                    ("CompanyA.Graphic.Builder.exe", 5),
                    ("graphic-builder", 1),
                ],
                "app.log",
                (10, 90),
            ),
            grp("graphic builder service", &[("GraphicBuilderService.exe", 30)], "svc.log", (10, 90)),
            grp("report builder", &[("reportbuilder.exe", 9)], "app.log", (10, 90)),
            grp("tray agent", &[("trayagent.exe", 40)], "app.log", (10, 90)),
        ]);
        b
    }

    fn resolve(b: &NameBook, ctx: &SearchContext, s: NameSettings, text: &str) -> Resolution {
        Resolver {
            book: b,
            vocab: &Vocabulary::builtin(),
            ctx,
            settings: s,
        }
        .resolve(text, 5, None)
    }

    fn quick(b: &NameBook, text: &str) -> Resolution {
        resolve(b, &SearchContext::default(), NameSettings::default(), text)
    }

    #[test]
    fn the_graphic_builder_spellings_all_find_one_entity() {
        let b = book();
        for q in [
            "graphic builder",
            "Graphic_Builder_v2",
            "graphicbuilder",
            "GraphicBuilder.exe",
            "Graphic.Builder.x64",
        ] {
            let r = quick(&b, q);
            assert_eq!(r.matches[0].id, "executable|graphic builder", "{q}");
            assert_eq!(r.matches[0].spellings.len(), 3);
        }
        let r = quick(&b, "graphic builder");
        assert_eq!(r.matches[0].how, How::Exact, "a dashed spelling has the same letters");
        let r = quick(&b, "Graphic Builder v2");
        assert_eq!(r.matches[0].how, How::Segmented);
        assert!(r.matches[0].why.contains("spelled differently"), "{}", r.matches[0].why);
        assert_eq!(r.matches[0].canonical, "graphicbuilder.exe");
        assert!(r.clarify.is_none());
        assert_eq!(quick(&b, "graphicbuilder.exe").matches[0].how, How::Exact);
    }

    #[test]
    fn typos_parts_and_prefixes_say_how() {
        let b = book();
        let t = quick(&b, "grafic bilder");
        assert_eq!(t.matches[0].how, How::Typo);
        assert!(t.matches[0].why.starts_with("close spelling"));
        let p = quick(&b, "tra");
        assert_eq!((p.matches[0].how, p.matches[0].id.as_str()), (How::Prefix, "executable|tray agent"));
        let part = quick(&b, "graphic builder service");
        assert_eq!(part.matches[0].how, How::Segmented);
        let none = quick(&b, "zzzz qqqq");
        assert!(none.matches.is_empty() && none.clarify.is_none());
        assert_eq!(quick(&b, "   ").matches.len(), 0);
    }

    #[test]
    fn tie_asks_a_question_and_context_breaks_it() {
        let b = book();
        let r = quick(&b, "builder");
        assert_eq!(r.matches[0].how, How::Parts);
        let c = r.clarify.expect("two builders tie");
        assert!(
            c.question.contains("Graphic Builder") && c.question.contains("(process)"),
            "{}",
            c.question
        );
        assert_eq!(c.options.len(), 3);
        // The inspected event mentions the report builder: context decides, no question.
        let ctx = SearchContext {
            inspected: Some("failed to start reportbuilder.exe".into()),
            ..SearchContext::default()
        };
        let r = resolve(&b, &ctx, NameSettings::default(), "builder");
        assert_eq!(r.matches[0].id, "executable|report builder");
        assert!(r.matches[0].why.contains("inspecting"), "{}", r.matches[0].why);
        assert!(r.clarify.is_none());
        // With the context switched off the tie is back.
        let off = NameSettings {
            context: false,
            ..NameSettings::default()
        };
        assert!(resolve(&b, &ctx, off, "builder").clarify.is_some());
    }

    #[test]
    fn context_ranks_by_window_source_and_recent_searches() {
        let mut b = NameBook::default();
        b.rebuild(vec![
            grp("alpha tool", &[("alphatool.exe", 10)], "one.log", (0, 10)),
            grp("beta tool", &[("betatool.exe", 10)], "two.log", (100, 200)),
        ]);
        let ctx = SearchContext {
            window: Some((120, 150)),
            sources: vec!["two.log".into()],
            ..SearchContext::default()
        };
        let r = resolve(&b, &ctx, NameSettings::default(), "tool");
        assert_eq!(r.matches[0].id, "executable|beta tool");
        assert!(r.matches[0].why.contains("time window") && r.matches[0].why.contains("open source"));
        assert!(r.clarify.is_none());
    }

    #[test]
    fn cooccurrence_with_an_anchor_breaks_a_tie() {
        let b = book();
        let ctx = SearchContext {
            anchors: vec!["trayagent.exe".into()],
            ..SearchContext::default()
        };
        let mut co = Cooccurrence::default();
        for _ in 0..3 {
            co.add_event(&["reportbuilder.exe".to_string(), "trayagent.exe".to_string()]);
        }
        let r = Resolver {
            book: &b,
            vocab: &Vocabulary::builtin(),
            ctx: &ctx,
            settings: NameSettings::default(),
        }
        .resolve("builder", 5, Some(&co));
        assert_eq!(r.matches[0].id, "executable|report builder");
        assert!(r.matches[0].why.contains("same events as trayagent.exe"));
        assert!(r.clarify.is_none());
    }

    #[test]
    fn ai_aliases_match_and_can_be_switched_off() {
        let mut b = book();
        b.set_ai(
            "executable|graphic builder",
            AiEntry {
                model: "m".into(),
                human: "Graphic Builder".into(),
                aliases: vec!["image editor".into()],
            },
        );
        let r = quick(&b, "Image Editor");
        assert_eq!(r.matches[0].how, How::Alias);
        assert!(r.matches[0].why.starts_with("AI alias"), "{}", r.matches[0].why);
        let off = NameSettings {
            ai_aliases: false,
            ..NameSettings::default()
        };
        assert!(resolve(&b, &SearchContext::default(), off, "image editor").matches.is_empty());
    }

    #[test]
    fn user_feedback_outranks_ai_and_rejections_hide() {
        let mut b = book();
        b.set_ai(
            "executable|graphic builder",
            AiEntry {
                model: "m".into(),
                human: "x".into(),
                aliases: vec!["editor".into()],
            },
        );
        b.set_ai(
            "executable|report builder",
            AiEntry {
                model: "m".into(),
                human: "y".into(),
                aliases: vec!["editor".into()],
            },
        );
        // The model gave the same alias to two groups: a tie.
        assert!(quick(&b, "editor").clarify.is_some());
        // The user accepts one: it wins, no question, and the AI alias of the other no longer competes.
        b.accept("editor", "executable|report builder", false).unwrap();
        let r = quick(&b, "editor");
        assert_eq!(r.matches[0].id, "executable|report builder");
        assert_eq!(r.matches[0].how, How::UserAlias);
        assert!(r.clarify.is_none());
        // A rejection removes the AI match for that group.
        b.reject("editor", "executable|graphic builder", false).unwrap();
        assert!(quick(&b, "editor").matches.iter().all(|m| m.id != "executable|graphic builder"));
        // Undo restores it.
        b.undo();
        assert!(quick(&b, "editor").matches.iter().any(|m| m.id == "executable|graphic builder"));
    }

    #[test]
    fn typos_only_when_nothing_matches_better() {
        let b = book();
        let r = quick(&b, "trayagnt");
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].how, How::Typo);
    }

    #[test]
    fn edit_distance_is_capped() {
        assert_eq!(edit_distance_capped("kitten", "sitting", 3), Some(3));
        assert_eq!(edit_distance_capped("kitten", "sitting", 2), None);
        assert_eq!(edit_distance_capped("a", "abcdef", 2), None);
        assert_eq!(edit_distance_capped("", "", 1), Some(0));
        assert_eq!(edit_distance_capped("cahce", "cache", 1), Some(1), "a swap is one edit");
    }

    #[test]
    fn kind_words_and_hows() {
        assert_eq!(kind_word(&Scope::Field("ProcessName".into())), "ProcessName");
        assert_eq!(kind_word(&Scope::Entity(EntityKind::Library)), "library");
        assert_eq!(kind_word(&Scope::Entity(EntityKind::File)), "file");
        assert_eq!(kind_word(&Scope::Entity(EntityKind::Exception)), "exception");
        assert_eq!(kind_word(&Scope::Entity(EntityKind::Host)), "host");
        let ids: Vec<&str> = [
            How::UserAlias,
            How::Exact,
            How::Segmented,
            How::Parts,
            How::Prefix,
            How::Alias,
            How::Typo,
        ]
        .iter()
        .map(|h| h.id())
        .collect();
        assert_eq!(ids.len(), 7);
    }
}
