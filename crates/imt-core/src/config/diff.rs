//! Path-based diff of two configuration trees.
//!
//! Nodes are matched by path (case-insensitively when either tree or the
//! options ask for it); lists are compared by position. The result lists the
//! union of both trees in tree order (sorted by path, parents before their
//! children), each entry added, removed, changed (old → new) or unchanged.
//! Paths matching an ignore pattern are left out with their subtrees.

use super::dto::{ChangeKind, CompareOptions, DiffCounts, DiffRow, DiffView, ExpandMode, ValueDto, MAX_IGNORE_PATTERNS, MAX_PAGE};
use super::tree::display_path;
use super::{ConfigTree, ConfigValue, PathSeg};
use anyhow::{anyhow, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::Regex;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

/// Volatile values that change on every snapshot of the same machine
/// (install and write times, timestamps, boot time), ignored by default.
pub const DEFAULT_IGNORE: &[&str] = &[
    "*InstallDate*",
    "*InstallTime*",
    "*LastWriteTime*",
    "*LastModified*",
    "*LastUpdated*",
    "*LastBootUpTime*",
    "*Timestamp*",
];

/// Longest ignore pattern accepted.
const MAX_PATTERN: usize = 1024;

/// Compiled ignore patterns: globs, or regexes written `re:<pattern>`; all
/// ignore case and match the whole display path.
#[derive(Debug, Clone)]
pub struct IgnoreSet {
    globs: GlobSet,
    regexes: Vec<Regex>,
}

impl IgnoreSet {
    pub fn new(patterns: &[String]) -> Result<IgnoreSet> {
        if patterns.len() > MAX_IGNORE_PATTERNS {
            return Err(anyhow!("too many ignore patterns (limit {MAX_IGNORE_PATTERNS})"));
        }
        let mut globs = GlobSetBuilder::new();
        let mut regexes = Vec::new();
        for p in patterns.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            if p.len() > MAX_PATTERN {
                return Err(anyhow!("an ignore pattern is longer than {MAX_PATTERN} characters"));
            }
            match p.strip_prefix("re:") {
                Some(re) => regexes.push(crate::user_regex::compile(re, true).map_err(|e| anyhow!("ignore pattern \"{p}\": {e}"))?),
                None => {
                    let glob = GlobBuilder::new(p)
                        .case_insensitive(true)
                        .literal_separator(false)
                        .build()
                        .map_err(|e| anyhow!("ignore pattern \"{p}\": {e}"))?;
                    globs.add(glob);
                }
            }
        }
        Ok(IgnoreSet {
            globs: globs.build().map_err(|e| anyhow!("ignore patterns: {e}"))?,
            regexes,
        })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.globs.is_match(path) || self.regexes.iter().any(|r| r.is_match(path))
    }
}

/// One node of the diff.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    /// Path as written in the right file (else the left one).
    pub path: Vec<PathSeg>,
    pub display: String,
    pub kind: ChangeKind,
    pub old: Option<ConfigValue>,
    pub new: Option<ConfigValue>,
    /// Something below changed.
    pub has_changes: bool,
}

impl DiffEntry {
    fn depth(&self) -> usize {
        self.path.len().saturating_sub(1)
    }
}

/// The diff of two trees.
#[derive(Debug, Clone)]
pub struct Diff {
    pub entries: Vec<DiffEntry>,
    pub counts: DiffCounts,
    pub case_insensitive: bool,
    /// Either tree reached the node limit.
    pub truncated: bool,
}

type Index = BTreeMap<Vec<PathSeg>, (Vec<PathSeg>, ConfigValue)>;

fn normalise(path: &[PathSeg], ci: bool) -> Vec<PathSeg> {
    path.iter()
        .map(|s| match s {
            PathSeg::Key(k) if ci => PathSeg::Key(k.to_lowercase()),
            other => other.clone(),
        })
        .collect()
}

/// Nodes by normalised path (a later duplicate wins), with missing ancestors added as sections.
fn index(tree: &ConfigTree, ci: bool) -> Index {
    let mut map = Index::new();
    for n in &tree.nodes {
        map.insert(normalise(&n.path, ci), (n.path.clone(), n.value.clone()));
    }
    let keys: Vec<(Vec<PathSeg>, Vec<PathSeg>)> = map.iter().map(|(k, (orig, _))| (k.clone(), orig.clone())).collect();
    for (norm, orig) in keys {
        for len in (1..norm.len()).rev() {
            if map.contains_key(&norm[..len]) {
                break;
            }
            map.insert(norm[..len].to_vec(), (orig[..len].to_vec(), ConfigValue::Section));
        }
    }
    map
}

/// Compares two trees.
pub fn diff(left: &ConfigTree, right: &ConfigTree, options: &CompareOptions) -> Result<Diff> {
    let ignore = IgnoreSet::new(&options.ignore)?;
    let ci = left.case_insensitive || right.case_insensitive || options.case_insensitive;
    let (l, r) = (index(left, ci), index(right, ci));
    let mut counts = DiffCounts::default();
    let mut entries = Vec::new();
    let mut li = l.into_iter().peekable();
    let mut ri = r.into_iter().peekable();
    let mut ignored: Option<Vec<PathSeg>> = None;
    while li.peek().is_some() || ri.peek().is_some() {
        let order = match (li.peek(), ri.peek()) {
            (Some((a, _)), Some((b, _))) => a.cmp(b),
            (Some(_), None) => Ordering::Less,
            _ => Ordering::Greater,
        };
        let (old, new) = match order {
            Ordering::Less => (li.next(), None),
            Ordering::Greater => (None, ri.next()),
            Ordering::Equal => (li.next(), ri.next()),
        };
        let norm = old.as_ref().or(new.as_ref()).map(|(k, _)| k.clone()).unwrap_or_default();
        let (old, new) = (old.map(|(_, v)| v), new.map(|(_, v)| v));
        let both = usize::from(old.is_some()) + usize::from(new.is_some());
        if ignored.as_ref().is_some_and(|p| norm.starts_with(p)) {
            counts.ignored += both;
            continue;
        }
        let path = new.as_ref().or(old.as_ref()).map(|(p, _)| p.clone()).unwrap_or_default();
        let display = display_path(&path);
        if ignore.matches(&display) {
            counts.ignored += both;
            ignored = Some(norm);
            continue;
        }
        let (old, new) = (old.map(|(_, v)| v), new.map(|(_, v)| v));
        let kind = match (&old, &new) {
            (Some(a), Some(b)) if a == b => ChangeKind::Unchanged,
            (Some(_), Some(_)) => ChangeKind::Changed,
            (Some(_), None) => ChangeKind::Removed,
            _ => ChangeKind::Added,
        };
        match kind {
            ChangeKind::Added => counts.added += 1,
            ChangeKind::Removed => counts.removed += 1,
            ChangeKind::Changed => counts.changed += 1,
            ChangeKind::Unchanged => counts.unchanged += 1,
        }
        entries.push(DiffEntry {
            path,
            display,
            kind,
            old,
            new,
            has_changes: false,
        });
    }
    mark_changes(&mut entries);
    Ok(Diff {
        entries,
        counts,
        case_insensitive: ci,
        truncated: left.truncated || right.truncated,
    })
}

/// Flags every ancestor of a changed entry (entries are in tree order with all ancestors present).
fn mark_changes(entries: &mut [DiffEntry]) {
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..entries.len() {
        let depth = entries[i].depth();
        while stack.last().is_some_and(|&a| entries[a].depth() >= depth) {
            stack.pop();
        }
        if entries[i].kind != ChangeKind::Unchanged {
            for &a in stack.iter().rev() {
                if entries[a].has_changes {
                    break;
                }
                entries[a].has_changes = true;
            }
        }
        stack.push(i);
    }
}

impl Diff {
    /// Whether an entry passes the view's kind and search filters.
    pub fn matches(&self, e: &DiffEntry, kinds: &[ChangeKind], search: &str) -> bool {
        if !kinds.contains(&e.kind) {
            return false;
        }
        if search.is_empty() {
            return true;
        }
        let hit = |s: &str| s.to_lowercase().contains(search);
        hit(&e.display) || [&e.old, &e.new].iter().any(|v| v.as_ref().is_some_and(|v| hit(&v.text())))
    }

    /// The rows a view shows (matching entries, their ancestors as context,
    /// collapsed sections folded), and the page of them it asks for.
    pub fn rows(&self, view: &DiffView) -> (usize, Vec<DiffRow>) {
        let search = view.search.trim().to_lowercase();
        // 0 = hidden, 1 = context, 2 = match.
        let mut shown = vec![0u8; self.entries.len()];
        let mut stack: Vec<usize> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            while stack.last().is_some_and(|&a| self.entries[a].depth() >= e.depth()) {
                stack.pop();
            }
            if self.matches(e, &view.kinds, &search) {
                shown[i] = 2;
                for &a in stack.iter().rev() {
                    if shown[a] != 0 {
                        break;
                    }
                    shown[a] = 1;
                }
            }
            stack.push(i);
        }
        let included: Vec<usize> = (0..self.entries.len()).filter(|&i| shown[i] != 0).collect();
        let toggled: HashSet<&str> = view.toggled.iter().map(String::as_str).collect();
        let mut visible: Vec<(usize, bool, bool)> = Vec::new();
        let mut fold_below: Option<usize> = None;
        for (pos, &i) in included.iter().enumerate() {
            let e = &self.entries[i];
            if fold_below.is_some_and(|d| e.depth() > d) {
                continue;
            }
            fold_below = None;
            let expandable = included.get(pos + 1).is_some_and(|&n| self.entries[n].depth() > e.depth());
            let by_default = !search.is_empty() || view.expand == ExpandMode::All || e.has_changes;
            let expanded = expandable && (by_default != toggled.contains(e.display.as_str()));
            if expandable && !expanded {
                fold_below = Some(e.depth());
            }
            visible.push((i, expandable, expanded));
        }
        let total = visible.len();
        let limit = view.limit.clamp(1, MAX_PAGE);
        let start = view.offset.min(total);
        let rows = visible[start..(start + limit).min(total)]
            .iter()
            .map(|&(i, expandable, expanded)| {
                let e = &self.entries[i];
                DiffRow {
                    path: e.display.clone(),
                    name: e.path.last().map(PathSeg::label).unwrap_or_default(),
                    depth: e.depth(),
                    kind: e.kind,
                    old: e.old.as_ref().map(ValueDto::from),
                    new: e.new.as_ref().map(ValueDto::from),
                    has_changes: e.has_changes,
                    expandable,
                    expanded,
                    context: shown[i] == 1,
                }
            })
            .collect();
        (total, rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigParser;
    use crate::config::{ConfigNode, IniParser, JsonParser, Limits};

    fn json(s: &str) -> ConfigTree {
        JsonParser.parse(s, &Limits::default()).unwrap()
    }

    fn no_ignore() -> CompareOptions {
        CompareOptions {
            ignore: Vec::new(),
            case_insensitive: false,
        }
    }

    fn summary(d: &Diff) -> Vec<String> {
        d.entries
            .iter()
            .map(|e| {
                let v = |v: &Option<ConfigValue>| v.as_ref().map_or("-".to_string(), |v| v.text());
                format!("{} {} {}→{}", e.kind.as_str(), e.display, v(&e.old), v(&e.new))
            })
            .collect()
    }

    #[test]
    fn added_removed_changed_nested_and_arrays() {
        let d = diff(
            &json(r#"{"a": 1, "b": {"c": "x", "d": [1, 2, 3]}, "gone": {"x": 1}, "t": 1, "zz": 0}"#),
            &json(r#"{"a": 2, "b": {"c": "x", "d": [1, 5]}, "new": true, "t": "1"}"#),
            &no_ignore(),
        )
        .unwrap();
        assert_eq!(
            summary(&d),
            [
                "changed a 1→2",
                "unchanged b →",
                "unchanged b/c x→x",
                "unchanged b/d →",
                "unchanged b/d[0] 1→1",
                "changed b/d[1] 2→5",
                "removed b/d[2] 3→-",
                "removed gone →-",
                "removed gone/x 1→-",
                "added new -→true",
                "changed t 1→1",
                "removed zz 0→-",
            ]
        );
        assert_eq!(
            (d.counts.added, d.counts.removed, d.counts.changed, d.counts.unchanged),
            (1, 4, 3, 4)
        );
        let b = d.entries.iter().find(|e| e.display == "b").unwrap();
        assert!(b.has_changes);
        assert!(d.entries.iter().find(|e| e.display == "b/c").is_some_and(|e| !e.has_changes));
        assert!(!d.truncated && !d.case_insensitive);
    }

    #[test]
    fn case_insensitive_paths_and_implied_ancestors() {
        let mut left = ConfigTree {
            format: "registry".into(),
            case_insensitive: true,
            truncated: false,
            nodes: vec![ConfigNode {
                path: vec![PathSeg::key("HKLM"), PathSeg::key("Software"), PathSeg::key("Value")],
                value: ConfigValue::Integer(1),
            }],
        };
        let right = ConfigTree {
            nodes: vec![ConfigNode {
                path: vec![PathSeg::key("hklm"), PathSeg::key("SOFTWARE"), PathSeg::key("value")],
                value: ConfigValue::Integer(1),
            }],
            truncated: true,
            ..left.clone()
        };
        let d = diff(&left, &right, &no_ignore()).unwrap();
        assert_eq!(
            summary(&d),
            ["unchanged hklm →", "unchanged hklm/SOFTWARE →", "unchanged hklm/SOFTWARE/value 1→1"]
        );
        assert!(d.case_insensitive && d.truncated);
        // Case-sensitive trees with the option set.
        left.case_insensitive = false;
        let r2 = ConfigTree {
            case_insensitive: false,
            ..right.clone()
        };
        assert_eq!(diff(&left, &r2, &no_ignore()).unwrap().counts.unchanged, 0);
        let opts = CompareOptions {
            case_insensitive: true,
            ..no_ignore()
        };
        assert_eq!(diff(&left, &r2, &opts).unwrap().counts.unchanged, 3);
    }

    #[test]
    fn ignore_patterns_drop_subtrees() {
        let left = json(r#"{"InstallDate": 1, "app": {"cache": {"x": 1}, "v": 1}, "id": "a"}"#);
        let right = json(r#"{"InstallDate": 2, "app": {"cache": {"x": 2, "y": 3}, "v": 1}, "id": "b"}"#);
        let opts = CompareOptions {
            ignore: vec!["*installdate*".into(), "app/cache".into(), "re:^i[d]$".into(), "  ".into()],
            case_insensitive: false,
        };
        let d = diff(&left, &right, &opts).unwrap();
        assert_eq!(summary(&d), ["unchanged app →", "unchanged app/v 1→1"]);
        assert_eq!(d.counts.ignored, 2 + 2 + 2 + 1 + 2);
        // The defaults ignore install dates.
        let d = diff(&left, &right, &CompareOptions::default()).unwrap();
        assert!(d.entries.iter().all(|e| e.display != "InstallDate"));
    }

    #[test]
    fn bad_ignore_patterns_fail_fast() {
        let bad = |p: &str| IgnoreSet::new(&[p.to_string()]).unwrap_err().to_string();
        assert!(bad("re:(").contains("re:("));
        assert!(bad("a[").contains("a["));
        assert!(bad(&"x".repeat(2000)).contains("longer"));
        assert!(IgnoreSet::new(&vec!["a".to_string(); MAX_IGNORE_PATTERNS + 1])
            .unwrap_err()
            .to_string()
            .contains("too many"));
        let t = json("{}");
        assert!(diff(
            &t,
            &t,
            &CompareOptions {
                ignore: vec!["re:(".into()],
                case_insensitive: false
            }
        )
        .is_err());
    }

    #[test]
    fn views_filter_search_and_fold() {
        let left = IniParser.parse("[a]\nx=1\ny=2\n[b]\nz=1\n[c]\nk=1\n", &Limits::default()).unwrap();
        let right = IniParser.parse("[a]\nx=1\ny=3\n[b]\nz=1\n[d]\nk=1\n", &Limits::default()).unwrap();
        let d = diff(&left, &right, &no_ignore()).unwrap();
        let paths = |v: &DiffView| d.rows(v).1.into_iter().map(|r| r.path).collect::<Vec<_>>();
        let all = DiffView::default();
        assert_eq!(paths(&all), ["a", "a/x", "a/y", "b", "b/z", "c", "c/k", "d", "d/k"]);
        // Changes only: unchanged ancestors become context.
        let changes = DiffView {
            kinds: vec![ChangeKind::Changed],
            ..DiffView::default()
        };
        let (total, rows) = d.rows(&changes);
        assert_eq!(
            (total, rows[0].context, rows[1].context, rows[1].path.as_str()),
            (2, true, false, "a/y")
        );
        assert_eq!(rows[1].old.as_ref().unwrap().text, "2");
        // Search matches paths and values, case-insensitively.
        let search = DiffView {
            search: " K ".into(),
            ..DiffView::default()
        };
        assert_eq!(paths(&search), ["c", "c/k", "d", "d/k"]);
        let by_value = DiffView {
            search: "3".into(),
            ..DiffView::default()
        };
        assert_eq!(paths(&by_value), ["a", "a/y"]);
        // Changed-only expansion folds unchanged sections; toggles flip that.
        let folded = DiffView {
            expand: ExpandMode::ChangedOnly,
            ..DiffView::default()
        };
        assert_eq!(paths(&folded), ["a", "a/x", "a/y", "b", "c", "c/k", "d", "d/k"]);
        let rows = d.rows(&folded).1;
        assert!(rows[3].expandable && !rows[3].expanded && !rows[3].has_changes);
        assert!(!rows[1].expandable && !rows[1].expanded);
        let toggled = DiffView {
            expand: ExpandMode::ChangedOnly,
            toggled: vec!["b".into(), "a".into()],
            ..DiffView::default()
        };
        assert_eq!(paths(&toggled), ["a", "b", "b/z", "c", "c/k", "d", "d/k"]);
        // Paging.
        let page = DiffView {
            offset: 7,
            limit: 5,
            ..DiffView::default()
        };
        let (total, rows) = d.rows(&page);
        assert_eq!(
            (total, rows.len(), rows[0].path.as_str(), rows[0].name.as_str(), rows[0].depth),
            (9, 2, "d", "d", 0)
        );
        let beyond = DiffView {
            offset: 100,
            limit: 0,
            ..DiffView::default()
        };
        assert_eq!(d.rows(&beyond), (9, vec![]));
    }
}
