//! A document on the wire (ADR 0017, ADR 0023): the dumps plugin's process sends the result of an import
//! (document or typed error) encoded with [`encode_result`]; Debrief decodes it with [`decode_result`], which checks
//! every count, index and text against [`Limits`] before the document reaches a view.

use crate::error::{DumpError, ErrorCode, Result};
use crate::limits::Limits;
use crate::model::{Document, Dump, UmdhLog};
use serde::{Deserialize, Serialize};

// One per import (see `Document`).
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize)]
enum WResult {
    Document(Document),
    Error { code: ErrorCode, message: String },
}

/// The bytes the plugin sends for an import's outcome.
pub fn encode_result(result: &Result<Document>) -> Vec<u8> {
    let w = match result {
        Ok(d) => WResult::Document(d.clone()),
        Err(e) => WResult::Error {
            code: e.code,
            message: e.message.clone(),
        },
    };
    postcard::to_allocvec(&w).unwrap_or_else(|e| {
        // A model that cannot be encoded is a bug; send it as an error rather than nothing.
        postcard::to_allocvec(&WResult::Error {
            code: ErrorCode::Invalid,
            message: format!("the document could not be encoded: {e}"),
        })
        .unwrap_or_default()
    })
}

/// The document (or the plugin's typed error) from `bytes`, checked against `limits`.
pub fn decode_result(bytes: &[u8], limits: &Limits) -> Result<Document> {
    if bytes.len() as u64 > limits.max_encoded_bytes {
        return Err(DumpError::limit(format!(
            "the document is {} MB; at most {} MB are accepted",
            bytes.len() >> 20,
            limits.max_encoded_bytes >> 20
        )));
    }
    let (w, rest): (WResult, &[u8]) =
        postcard::take_from_bytes(bytes).map_err(|e| DumpError::invalid(format!("the dumps plugin sent a damaged document: {e}")))?;
    if !rest.is_empty() {
        return Err(DumpError::invalid("the dumps plugin sent trailing bytes"));
    }
    match w {
        WResult::Error { code, message } => Err(DumpError::new(code, message)),
        WResult::Document(d) => {
            check(&d, limits)?;
            Ok(d)
        }
    }
}

fn bad(what: &str) -> DumpError {
    DumpError::invalid(format!("the dumps plugin sent an inconsistent document: {what}"))
}

fn text(s: &str, limits: &Limits, what: &str) -> Result<()> {
    if s.chars().count() > limits.max_text {
        return Err(bad(&format!("{what} is too long")));
    }
    Ok(())
}

fn opt_text(s: &Option<String>, limits: &Limits, what: &str) -> Result<()> {
    s.as_deref().map_or(Ok(()), |s| text(s, limits, what))
}

/// Checks the bounds and indices the views rely on.
pub fn check(d: &Document, limits: &Limits) -> Result<()> {
    match d {
        Document::Dump(d) => check_dump(d, limits),
        Document::Umdh(u) => check_umdh(u, limits),
    }
}

fn check_dump(d: &Dump, l: &Limits) -> Result<()> {
    if d.threads.len() > l.max_threads || d.modules.len() > l.max_modules || d.unloaded_modules.len() > l.max_modules {
        return Err(bad("too many threads or modules"));
    }
    if d.notes.len() > l.max_notes {
        return Err(bad("too many notes"));
    }
    for n in &d.notes {
        text(n, l, "a note")?;
    }
    for m in &d.modules {
        text(&m.path, l, "a module path")?;
        opt_text(&m.version, l, "a module version")?;
        opt_text(&m.debug_file, l, "a debug file")?;
        opt_text(&m.debug_id, l, "a debug id")?;
        if let Some(d) = m.symbols.detail() {
            text(d, l, "a symbol status")?;
        }
    }
    for m in &d.unloaded_modules {
        text(m, l, "an unloaded module")?;
    }
    let modules = d.modules.len() as u32;
    for t in &d.threads {
        if t.frames.len() > l.max_frames {
            return Err(bad("a thread has too many frames"));
        }
        opt_text(&t.name, l, "a thread name")?;
        for f in &t.frames {
            if f.module.is_some_and(|m| m >= modules) {
                return Err(bad("a frame names a module that does not exist"));
            }
            opt_text(&f.function, l, "a function name")?;
            opt_text(&f.file, l, "a source file")?;
        }
    }
    if let Some(e) = &d.exception {
        if e.parameters.len() > 15 {
            return Err(bad("the exception has too many parameters"));
        }
    }
    if let Some(h) = &d.handles {
        if h.by_type.len() > l.max_handle_types + 1 {
            return Err(bad("too many handle types"));
        }
        for (t, _) in &h.by_type {
            text(t, l, "a handle type")?;
        }
    }
    if let Some(s) = &d.system {
        text(&s.os, l, "the OS")?;
        text(&s.os_version, l, "the OS version")?;
        opt_text(&s.os_build, l, "the OS build")?;
        opt_text(&s.cpu_info, l, "the CPU")?;
    }
    if let Some(p) = &d.process {
        text(&p.name, l, "the process name")?;
        opt_text(&p.path, l, "the process path")?;
    }
    if let Some(r) = &d.dotnet {
        text(&r.module, l, "the runtime module")?;
        opt_text(&r.version, l, "the runtime version")?;
    }
    Ok(())
}

fn check_umdh(u: &UmdhLog, l: &Limits) -> Result<()> {
    if u.stacks.len() > l.max_stacks || u.modules.len() > l.max_modules || u.notes.len() > l.max_notes {
        return Err(bad("too many stacks, modules or notes"));
    }
    for n in &u.notes {
        text(n, l, "a note")?;
    }
    for m in &u.modules {
        text(&m.name, l, "a module name")?;
    }
    for s in &u.stacks {
        if s.frames.len() > l.max_umdh_frames {
            return Err(bad("an allocation stack has too many frames"));
        }
        for f in &s.frames {
            opt_text(&f.module, l, "a module name")?;
            opt_text(&f.function, l, "a function name")?;
            opt_text(&f.file, l, "a source file")?;
        }
    }
    if let Some(p) = &u.process {
        opt_text(&p.name, l, "the process name")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::testing::{sample_dump, sample_umdh};

    #[test]
    fn documents_and_errors_roundtrip() {
        let l = Limits::default();
        let d = Document::Dump(sample_dump());
        assert_eq!(decode_result(&encode_result(&Ok(d.clone())), &l).unwrap(), d);
        let u = Document::Umdh(sample_umdh(UmdhKind::Snapshot, 1, &[(1, 100, 2)]));
        assert_eq!(decode_result(&encode_result(&Ok(u.clone())), &l).unwrap(), u);
        let e = decode_result(&encode_result(&Err(DumpError::new(ErrorCode::NotADump, "nope"))), &l).unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::NotADump, "nope"));
    }

    #[test]
    fn damaged_or_inconsistent_documents_are_refused() {
        let l = Limits::default();
        assert_eq!(decode_result(b"\xff\xff\xff", &l).unwrap_err().code, ErrorCode::Invalid);
        let mut bytes = encode_result(&Ok(Document::Dump(sample_dump())));
        bytes.push(0);
        assert!(decode_result(&bytes, &l).unwrap_err().message.contains("trailing"));
        let small = Limits { max_encoded_bytes: 4, ..l };
        assert_eq!(
            decode_result(&encode_result(&Ok(Document::Dump(sample_dump()))), &small)
                .unwrap_err()
                .code,
            ErrorCode::Limit
        );

        let mut d = sample_dump();
        d.threads[0].frames[0].module = Some(99);
        assert!(check(&Document::Dump(d), &l).unwrap_err().message.contains("module"));
        let mut d = sample_dump();
        d.modules[0].path = "x".repeat(l.max_text + 1);
        assert!(check(&Document::Dump(d), &l).is_err());
        let mut d = sample_dump();
        d.exception.as_mut().unwrap().parameters = vec![0; 16];
        assert!(check(&Document::Dump(d), &l).is_err());
        let mut d = sample_dump();
        d.threads[0].frames = vec![d.threads[0].frames[0].clone(); l.max_frames + 1];
        assert!(check(&Document::Dump(d), &l).is_err());
        let mut d = sample_dump();
        d.notes = vec![String::new(); l.max_notes + 1];
        assert!(check(&Document::Dump(d), &l).is_err());
        let mut d = sample_dump();
        d.handles = Some(HandleSummary {
            total: 1,
            by_type: vec![("x".into(), 1); l.max_handle_types + 2],
        });
        assert!(check(&Document::Dump(d), &l).is_err());

        let mut u = sample_umdh(UmdhKind::Diff, 1, &[(1, 100, 2)]);
        u.stacks[0].frames = vec![u.stacks[0].frames[0].clone(); l.max_umdh_frames + 1];
        assert!(check(&Document::Umdh(u), &l).is_err());
        let mut u = sample_umdh(UmdhKind::Diff, 1, &[(1, 100, 2)]);
        u.stacks[0].frames[0].function = Some("f".repeat(l.max_text + 1));
        assert!(check(&Document::Umdh(u), &l).is_err());
    }
}
