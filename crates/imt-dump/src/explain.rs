//! What "Explain with AI" sends to the chat: a compact, redacted summary of the documents and their findings.
//! No memory contents (the model has none), paths cut to file names (they hold user and machine names), thread
//! names left out, and the whole text bounded.

use crate::api::DumpFinding;
use crate::model::{Dump, UmdhLog};
use crate::modules::file_name;
use crate::views::{faulting_thread, frame_label};

/// Most characters sent.
pub const MAX_CHARS: usize = 6000;
/// Frames of the faulting thread included.
const STACK_FRAMES: usize = 16;
/// Findings included.
const MAX_FINDINGS: usize = 12;

fn findings_text(out: &mut Vec<String>, findings: &[DumpFinding]) {
    if findings.is_empty() {
        return;
    }
    out.push("Findings computed by IMT's dump engine:".into());
    for f in findings.iter().take(MAX_FINDINGS) {
        let severity = format!("{:?}", f.severity).to_lowercase();
        out.push(format!("- [{severity}] {}. {}", f.title, f.detail));
    }
}

fn bounded(lines: Vec<String>) -> String {
    let text = lines.join("\n");
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(MAX_CHARS - 1).collect();
    cut.push('\u{2026}');
    cut
}

/// The question for one dump.
pub fn dump_prompt(name: &str, d: &Dump, findings: &[DumpFinding]) -> String {
    let mut out = vec![format!(
        "Explain this crash dump in plain words: the likely root cause, how confident that is, and what to check or change first. Dump: {}.",
        file_name(name)
    )];
    let mut facts = Vec::new();
    if let Some(p) = &d.process {
        facts.push(format!("process {}", file_name(&p.name)));
    }
    if let Some(s) = &d.system {
        facts.push(format!("{} {} {}", s.os, s.os_version, s.arch.label()));
    }
    facts.push(format!("{} dump", crate::views::dump_kind(d.kind)));
    facts.push(format!("{} threads, {} modules", d.threads.len(), d.modules.len()));
    if let Some(r) = crate::views::dotnet_label(d) {
        facts.push(r);
    }
    out.push(format!("Facts: {}.", facts.join(", ")));
    if let Some(t) = faulting_thread(d) {
        out.push(format!("Faulting thread {} (innermost frame first):", t.id));
        for (i, f) in t.frames.iter().take(STACK_FRAMES).enumerate() {
            out.push(format!("  #{i} {}", frame_label(d, f)));
        }
        if t.frames.len() > STACK_FRAMES {
            out.push(format!("  … {} more frames", t.frames.len() - STACK_FRAMES));
        }
    }
    findings_text(&mut out, findings);
    bounded(out)
}

/// The question for a series of UMDH logs.
pub fn umdh_prompt(names: &[&str], logs: &[&UmdhLog], findings: &[DumpFinding]) -> String {
    let files: Vec<&str> = names.iter().map(|n| file_name(n)).collect();
    let mut out = vec![format!(
        "Explain these heap allocation logs (UMDH) in plain words: which code likely leaks memory, how sure that is, and what to look at first. Logs: {}.",
        files.join(", ")
    )];
    if let Some(p) = logs.iter().find_map(|l| l.process.as_ref()).and_then(|p| p.name.as_deref()) {
        out.push(format!("Process: {}.", file_name(p)));
    }
    let stacks: usize = logs.iter().map(|l| l.stacks.len()).sum();
    out.push(format!("{} log(s), {stacks} allocation stacks in all.", logs.len()));
    findings_text(&mut out, findings);
    bounded(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UmdhKind;
    use crate::testing::{sample_dump, sample_umdh};

    #[test]
    fn the_dump_summary_is_compact_and_redacted() {
        let d = sample_dump();
        let f = crate::findings::findings(1, &d);
        let p = dump_prompt("C:\\Users\\alice\\dumps\\OrderService.dmp", &d, &f);
        assert!(p.contains("Dump: OrderService.dmp."));
        assert!(!p.contains("alice") && !p.contains("Program Files"), "{p}");
        assert!(p.contains("#2 Shop.Orders.dll!Cache::Evict+0x40"));
        assert!(p.contains("[critical] ACCESS_VIOLATION"));
        assert!(!p.contains("worker-"), "thread names are left out");
        let mut deep = d.clone();
        let f0 = deep.threads[0].frames[0].clone();
        deep.threads[0].frames = vec![f0; 40];
        let long = dump_prompt("x.dmp", &deep, &[]);
        assert!(long.contains("… 24 more frames"));
        let mut huge = f.clone();
        huge[0].detail = "x".repeat(MAX_CHARS * 2);
        let cut = dump_prompt("x.dmp", &d, &huge);
        assert_eq!(cut.chars().count(), MAX_CHARS);
    }

    #[test]
    fn the_umdh_summary_names_files_only() {
        let a = sample_umdh(UmdhKind::Snapshot, 0, &[(1, 100, 1)]);
        let b = sample_umdh(UmdhKind::Snapshot, 1, &[(1, 5 << 20, 5)]);
        let f = crate::umdh::findings(&[(1, &a), (2, &b)]).unwrap();
        let p = umdh_prompt(&["/home/bob/a.txt", "/home/bob/b.txt"], &[&a, &b], &f);
        assert!(p.contains("Logs: a.txt, b.txt.") && !p.contains("bob"));
        assert!(p.contains("Process: OrderService.exe."));
        assert!(p.contains("Cache::Add"));
    }
}
