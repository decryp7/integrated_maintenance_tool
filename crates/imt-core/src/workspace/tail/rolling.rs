//! Rolling-file naming: the logical log a rotated or dated file belongs to.
//! `log-20261017.txt`, `log-20261017_001.txt` (Serilog), `app.log.1`
//! (log4net), `app.2026-10-17.0.log` (NLog archives) and `app.log.2.gz`
//! (logrotate) all collapse to the name of the live file family.

use regex::Regex;
use std::sync::OnceLock;

fn numbered() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // log4net / logrotate: "app.log.1"
    RE.get_or_init(|| Regex::new(r"^(.+\.[A-Za-z][A-Za-z0-9]*)\.\d{1,4}$").expect("valid regex"))
}

fn stamped() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // A date (yyyyMMdd, yyyy-MM-dd, optionally with hour/minute/second) or a
    // 10-digit yyyyMMddHH, optionally followed by a sequence ("_001", ".0").
    RE.get_or_init(|| {
        Regex::new(r"[-_.]?(?:\d{4}[-_.]?\d{2}[-_.]?\d{2}(?:[T_\-.]?\d{2}(?:[-_.]?\d{2}){0,2})?|\d{10})(?:[-_.]\d{1,4})?(\.[A-Za-z][A-Za-z0-9]*)?$")
            .expect("valid regex")
    })
}

fn sequence() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // NLog numbered archives: "app.1.log", Serilog size roll "log_001.txt".
    RE.get_or_init(|| Regex::new(r"(?:\.\d{1,4}|_\d{3})(\.[A-Za-z][A-Za-z0-9]*)$").expect("valid regex"))
}

/// The family name of a log file name: rotation numbers, date stamps and
/// compression suffixes removed. Files that are not rolled keep their name.
pub fn family(file_name: &str) -> String {
    let lower = file_name.to_ascii_lowercase();
    let mut name = file_name;
    for ext in [".gz", ".zip", ".bz2"] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            name = &name[..name.len() - ext.len()];
            break;
        }
    }
    let mut name = numbered().replace(name, "$1").into_owned();
    let stem_len = name.find('.').unwrap_or(name.len());
    if let Some(m) = stamped().find(&name) {
        // Keep at least one character of the base name ("20261017.log" stays).
        if m.start() > 0 && m.start() >= stem_len.min(1) {
            let ext = stamped().captures(&name).and_then(|c| c.get(1)).map(|e| e.as_str().to_string());
            name = format!("{}{}", &name[..m.start()], ext.unwrap_or_default());
        }
    }
    if let Some(c) = sequence().captures(&name) {
        let m = c.get(0).map_or(0, |m| m.start());
        if m > 0 {
            name = format!("{}{}", &name[..m], &c[1]);
        }
    }
    name
}

/// The group of a source from its display name: the directory part kept, the
/// file name reduced to its [`family`].
pub fn group_of(display_name: &str) -> String {
    match display_name.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/{}", family(file)),
        None => family(display_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families() {
        for (name, fam) in [
            ("app.log", "app.log"),
            ("app.log.1", "app.log"),
            ("app.log.12", "app.log"),
            ("app.log.2.gz", "app.log"),
            ("log-20261017.txt", "log.txt"),
            ("log-20261017_001.txt", "log.txt"),
            ("log-2026101710.txt", "log.txt"),
            ("log20261017.txt", "log.txt"),
            ("app.2026-10-17.log", "app.log"),
            ("app.2026-10-17.0.log", "app.log"),
            ("app-2026-10-17T10-30.log", "app.log"),
            ("app.1.log", "app.log"),
            ("log_001.txt", "log.txt"),
            ("web01.log", "web01.log"),
            ("error-log.txt", "error-log.txt"),
            ("20261017.log", "20261017.log"),
            ("x.zip", "x"),
            (".gz", ".gz"),
        ] {
            assert_eq!(family(name), fam, "{name}");
        }
        assert_eq!(group_of("logs/app.log.1"), "logs/app.log");
        assert_eq!(group_of("app.log.1"), "app.log");
    }
}
