//! Windows minidumps (`.dmp`, `MDMP` signature).
//!
//! A dump becomes a short summary, all records stamped with the time the dump was written:
//!
//! 1. `Kind=system`: OS name and version, CPU architecture, model and count.
//! 2. `Kind=process` (when the dump says): executable name and path, process id, start
//!    time and uptime.
//! 3. `Kind=exception`, level Error: the exception code with a readable name
//!    (`ACCESS_VIOLATION`), the faulting address, module + offset and thread id. Without an
//!    exception stream (a dump taken on request) this is an Information record instead.
//! 4. `Kind=frame`: the crashing thread's stack, frame 0 from the CPU context and the rest
//!    from a stack scan (`Trust=scan`): pointer-sized stack words that fall inside a loaded
//!    module, as debuggers do without symbols. Scanned frames are candidate return
//!    addresses, not a verified unwind.
//! 5. `Kind=module`: every loaded module with name, path, version, base and size.
//!
//! Parsing is done by the `minidump` crate (rust-minidump, fuzzed upstream); this module
//! only reads the streams it needs and caps everything it emits (frames, scanned bytes,
//! modules, string lengths), so time and memory stay bounded on hostile input.

use anyhow::{anyhow, bail, Result};
use debrief_plugin_api::importer::{BinaryImporter, ImportCtx, Imported};
use debrief_plugin_api::model::{Level, Record};
use minidump::{
    Error as DumpError, Minidump, MinidumpException, MinidumpMiscInfo, MinidumpModuleList, MinidumpSystemInfo, MinidumpThreadList, Module,
    UnifiedMemoryList,
};
use serde_json::Value;
use std::time::UNIX_EPOCH;

const MAGIC: &[u8] = b"MDMP";
/// At most this many stack frames are reported for the crashing thread.
pub const MAX_FRAMES: usize = 64;
/// At most this many stack bytes above the stack pointer are scanned for return addresses.
pub const MAX_SCAN_BYTES: usize = 256 * 1024;
/// At most this many modules are listed; the rest are counted as skipped.
pub const MAX_MODULES: usize = 4096;
/// Strings taken from the dump (paths, CPU names) are cut to this many characters.
const MAX_TEXT: usize = 1024;

/// True when the bytes start with the minidump signature.
pub fn is_minidump(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Imports Windows minidumps as a summary (see the module docs).
pub struct MinidumpImporter;

impl BinaryImporter for MinidumpImporter {
    fn id(&self) -> &'static str {
        "minidump"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &[u8]) -> u8 {
        if is_minidump(head) {
            100
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        read_minidump(bytes)
    }
}

/// Summarises a minidump as records. Fails with a readable message for anything that is
/// not a minidump; damaged optional streams are left out of the summary instead.
pub fn read_minidump(bytes: &[u8]) -> Result<Imported> {
    if !is_minidump(bytes) {
        bail!("not a Windows minidump (missing MDMP signature)");
    }
    let dump = Minidump::read(bytes).map_err(|e| match e {
        DumpError::VersionMismatch => anyhow!("unsupported minidump version"),
        DumpError::MissingHeader | DumpError::MissingDirectory => anyhow!("truncated minidump: the header or stream directory is cut off"),
        other => anyhow!("unreadable minidump: {other}"),
    })?;
    let ts = (dump.header.time_date_stamp != 0).then(|| i64::from(dump.header.time_date_stamp) * 1000);
    let system = dump.get_stream::<MinidumpSystemInfo>().ok();
    let misc = dump.get_stream::<MinidumpMiscInfo>().ok();
    let modules = dump.get_stream::<MinidumpModuleList>().ok();
    let threads = dump.get_stream::<MinidumpThreadList>().ok();
    let exception = dump.get_stream::<MinidumpException>().ok();
    let memory = dump.get_memory().unwrap_or_default();

    let mut out = Summary { records: Vec::new(), ts };
    let os = system.as_ref().map_or(minidump::system_info::Os::Unknown(0), |s| s.os);
    let cpu = system.as_ref().map_or(minidump::system_info::Cpu::Unknown(0), |s| s.cpu);
    if let Some(s) = &system {
        out.system(s);
    }
    out.process(modules.as_ref(), misc.as_ref(), dump.header.time_date_stamp);
    let mods = modules.as_ref();
    match &exception {
        Some(ex) => {
            let crash = Crash::of(ex, os, cpu, mods);
            let frames = system
                .as_ref()
                .map(|s| stack_frames(ex, s, misc.as_ref(), threads.as_ref(), &memory, mods))
                .unwrap_or_default();
            out.exception(&crash, &frames);
            out.frames(&frames, crash.thread_id);
        }
        None => {
            out.push(
                Level::Information,
                "No exception: the dump was written on request".into(),
                &[("Kind", "exception".into())],
            );
        }
    }
    let mut skipped = 0;
    if let Some(list) = mods {
        skipped = out.modules(list);
    }
    Ok(Imported {
        records: out.records,
        skipped,
        ..Default::default()
    })
}

/// Collects the summary records with increasing line numbers.
struct Summary {
    records: Vec<Record>,
    ts: Option<i64>,
}

impl Summary {
    fn push(&mut self, level: Level, message: String, fields: &[(&str, Value)]) -> &mut Record {
        let mut r = Record::new(self.records.len() as u32 + 1);
        r.ts = self.ts;
        r.level = level;
        r.message = message;
        for (k, v) in fields {
            r.fields.insert((*k).into(), v.clone());
        }
        self.records.push(r);
        let last = self.records.len() - 1;
        &mut self.records[last]
    }

    fn system(&mut self, s: &MinidumpSystemInfo) {
        let raw = &s.raw;
        let version = format!("{}.{}.{}", raw.major_version, raw.minor_version, raw.build_number);
        let csd = s.csd_version().map(|c| clip(&c)).filter(|c| !c.is_empty());
        let cpu_info = s.cpu_info().map(|c| clip(&c)).filter(|c| !c.is_empty());
        let mut msg = format!("{} {version}", s.os.long_name());
        if let Some(c) = &csd {
            msg.push_str(&format!(" ({c})"));
        }
        msg.push_str(&format!(", {}, {} CPUs", s.cpu, raw.number_of_processors));
        if let Some(c) = &cpu_info {
            msg.push_str(&format!(", {c}"));
        }
        let mut fields = vec![
            ("Kind", Value::from("system")),
            ("Os", Value::from(s.os.long_name().into_owned())),
            ("OsVersion", Value::from(version)),
            ("Cpu", Value::from(s.cpu.to_string())),
            ("Processors", Value::from(raw.number_of_processors)),
        ];
        if let Some(c) = csd {
            fields.push(("ServicePack", Value::from(c)));
        }
        if let Some(c) = cpu_info {
            fields.push(("CpuInfo", Value::from(c)));
        }
        self.push(Level::Information, msg, &fields);
    }

    fn process(&mut self, modules: Option<&MinidumpModuleList>, misc: Option<&MinidumpMiscInfo>, dump_time: u32) {
        let path = modules.and_then(|m| m.main_module()).map(|m| clip(&m.code_file()));
        let pid = misc.and_then(|m| m.raw.process_id().copied());
        let created = misc
            .and_then(|m| m.process_create_time())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        if path.is_none() && pid.is_none() && created.is_none() {
            return;
        }
        let name = path.as_deref().map(file_name).unwrap_or("unknown process").to_string();
        let mut msg = format!("Process {name}");
        let mut fields = vec![("Kind", Value::from("process")), ("ProcessName", Value::from(name))];
        if let Some(p) = path {
            fields.push(("ProcessPath", Value::from(p)));
        }
        if let Some(pid) = pid {
            msg.push_str(&format!(" (pid {pid})"));
            fields.push(("ProcessId", Value::from(pid)));
        }
        if let Some(c) = created.filter(|c| *c > 0) {
            if let Some(start) = chrono::DateTime::from_timestamp(c as i64, 0) {
                fields.push(("ProcessCreateTime", Value::from(start.to_rfc3339())));
            }
            if let Some(up) = u64::from(dump_time).checked_sub(c).filter(|_| dump_time != 0) {
                msg.push_str(&format!(", up {}", uptime(up)));
                fields.push(("UptimeSeconds", Value::from(up)));
            }
        }
        self.push(Level::Information, msg, &fields);
    }

    fn exception(&mut self, c: &Crash, frames: &[Frame]) {
        let code = format!("{:#010X}", c.code).replacen("0X", "0x", 1);
        let mut msg = format!("{} ({code})", c.name);
        if let Some(access) = &c.access {
            msg.push_str(&format!(" {access}"));
        }
        msg.push_str(&format!(" at {:#x}", c.address));
        let mut fields = vec![
            ("Kind", Value::from("exception")),
            ("ExceptionCode", Value::from(code)),
            ("ExceptionName", Value::from(c.name.clone())),
            ("Reason", Value::from(c.reason.clone())),
            ("Address", Value::from(hex(c.address))),
            ("ThreadId", Value::from(c.thread_id)),
        ];
        if let Some(a) = &c.access {
            fields.push(("Access", Value::from(a.clone())));
        }
        if let Some(loc) = &c.location {
            msg.push_str(&format!(" in {}", loc.label()));
            fields.push(("Module", Value::from(loc.module.clone())));
            fields.push(("Offset", Value::from(hex(loc.offset))));
        }
        msg.push_str(&format!(" on thread {}", c.thread_id));
        let r = self.push(Level::Error, msg, &fields);
        if !frames.is_empty() {
            r.exception = Some(frames.iter().map(Frame::line).collect::<Vec<_>>().join("\n"));
        }
    }

    fn frames(&mut self, frames: &[Frame], thread_id: u32) {
        for f in frames {
            let mut fields = vec![
                ("Kind", Value::from("frame")),
                ("Frame", Value::from(f.index)),
                ("Address", Value::from(hex(f.address))),
                ("Trust", Value::from(f.trust)),
                ("ThreadId", Value::from(thread_id)),
            ];
            if let Some(loc) = &f.location {
                fields.push(("Module", Value::from(loc.module.clone())));
                fields.push(("Offset", Value::from(hex(loc.offset))));
            }
            self.push(Level::Information, f.line(), &fields);
        }
    }

    /// Lists the modules by address; returns how many were left out over [`MAX_MODULES`].
    fn modules(&mut self, list: &MinidumpModuleList) -> usize {
        let mut skipped = 0;
        for (i, m) in list.by_addr().enumerate() {
            if i >= MAX_MODULES {
                skipped += 1;
                continue;
            }
            let path = clip(&m.code_file());
            let name = file_name(&path).to_string();
            let version = m.version().map(|v| v.into_owned());
            let mut msg = format!("Module {name}");
            if let Some(v) = &version {
                msg.push_str(&format!(" {v}"));
            }
            msg.push_str(&format!(" at {:#x}", m.base_address()));
            let mut fields = vec![
                ("Kind", Value::from("module")),
                ("Module", Value::from(name)),
                ("Path", Value::from(path)),
                ("Base", Value::from(hex(m.base_address()))),
                ("Size", Value::from(m.size())),
            ];
            if let Some(v) = version {
                fields.push(("Version", Value::from(v)));
            }
            self.push(Level::Information, msg, &fields);
        }
        skipped
    }
}

/// An address resolved to `module+offset`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Location {
    module: String,
    offset: u64,
}

impl Location {
    fn of(addr: u64, modules: Option<&MinidumpModuleList>) -> Option<Location> {
        let m = modules?.module_at_address(addr)?;
        Some(Location {
            module: file_name(&clip(&m.code_file())).to_string(),
            offset: addr.wrapping_sub(m.base_address()),
        })
    }

    fn label(&self) -> String {
        format!("{}+{:#x}", self.module, self.offset)
    }
}

/// The facts of the exception stream.
struct Crash {
    code: u32,
    name: String,
    reason: String,
    address: u64,
    access: Option<String>,
    location: Option<Location>,
    thread_id: u32,
}

impl Crash {
    fn of(
        ex: &MinidumpException,
        os: minidump::system_info::Os,
        cpu: minidump::system_info::Cpu,
        modules: Option<&MinidumpModuleList>,
    ) -> Crash {
        let rec = &ex.raw.exception_record;
        let reason = ex.get_crash_reason(os, cpu).to_string();
        let name = exception_name(rec.exception_code).map_or_else(|| reason.clone(), str::to_string);
        // The faulting instruction; for access violations `get_crash_address` is the data address.
        let ip = match cpu.pointer_width() {
            minidump::system_info::PointerWidth::Bits32 => rec.exception_address as u32 as u64,
            _ => rec.exception_address,
        };
        let access = (matches!(rec.exception_code, 0xC000_0005 | 0xC000_0006) && rec.number_parameters >= 2).then(|| {
            let target = ex.get_crash_address(os, cpu);
            let verb = match rec.exception_information[0] {
                0 => "reading",
                1 => "writing",
                8 => "executing",
                _ => "accessing",
            };
            format!("{verb} {target:#x}")
        });
        Crash {
            code: rec.exception_code,
            name,
            reason: clip(&reason),
            address: ip,
            access,
            location: Location::of(ip, modules),
            thread_id: ex.get_crashing_thread_id(),
        }
    }
}

/// One stack frame of the crashing thread.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    index: usize,
    address: u64,
    location: Option<Location>,
    /// `context` for the instruction pointer, `scan` for stack-scanned return addresses.
    trust: &'static str,
}

impl Frame {
    fn line(&self) -> String {
        match &self.location {
            Some(l) => format!("#{} {} ({:#x})", self.index, l.label(), self.address),
            None => format!("#{} {:#x}", self.index, self.address),
        }
    }
}

/// Frame 0 from the exception context (or the thread's own context), then stack-scanned
/// candidates: aligned words above the stack pointer that point into a loaded module.
fn stack_frames(
    ex: &MinidumpException,
    system: &MinidumpSystemInfo,
    misc: Option<&MinidumpMiscInfo>,
    threads: Option<&MinidumpThreadList>,
    memory: &UnifiedMemoryList,
    modules: Option<&MinidumpModuleList>,
) -> Vec<Frame> {
    let thread = threads.and_then(|t| t.get_thread(ex.get_crashing_thread_id()));
    let context = ex
        .context(system, misc)
        .map(|c| c.into_owned())
        .or_else(|| thread.and_then(|t| t.context(system, misc)).map(|c| c.into_owned()));
    let Some(context) = context else { return Vec::new() };
    let width: usize = match system.cpu.pointer_width() {
        minidump::system_info::PointerWidth::Bits32 => 4,
        _ => 8,
    };
    let ip = context.get_instruction_pointer();
    let mut frames = vec![Frame {
        index: 0,
        address: ip,
        location: Location::of(ip, modules),
        trust: "context",
    }];
    let Some(stack) = thread.and_then(|t| t.stack_memory(memory)) else {
        return frames;
    };
    let words = scan_words(stack.bytes(), stack.base_address(), context.get_stack_pointer(), width);
    for addr in words {
        if frames.len() >= MAX_FRAMES {
            break;
        }
        if let Some(loc) = Location::of(addr, modules) {
            frames.push(Frame {
                index: frames.len(),
                address: addr,
                location: Some(loc),
                trust: "scan",
            });
        }
    }
    frames
}

/// The little-endian pointer-sized words of `bytes` (mapped at `base`) from `sp` upwards,
/// at most [`MAX_SCAN_BYTES`] of them. Empty when `sp` lies outside the memory.
fn scan_words(bytes: &[u8], base: u64, sp: u64, width: usize) -> impl Iterator<Item = u64> + '_ {
    let start = sp
        .checked_sub(base)
        .and_then(|o| usize::try_from(o).ok())
        .filter(|o| *o < bytes.len())
        .unwrap_or(bytes.len());
    // Align to the pointer size relative to the region (stacks are pointer-aligned).
    let start = start.next_multiple_of(width).min(bytes.len());
    let end = bytes.len().min(start.saturating_add(MAX_SCAN_BYTES));
    bytes[start..end].chunks_exact(width).map(move |w| {
        let mut buf = [0u8; 8];
        buf[..width].copy_from_slice(w);
        u64::from_le_bytes(buf)
    })
}

/// The readable name of common Windows exception codes (`ACCESS_VIOLATION`), as shown by debuggers.
pub fn exception_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0xC000_0005 => "ACCESS_VIOLATION",
        0xC000_0006 => "IN_PAGE_ERROR",
        0xC000_0008 => "INVALID_HANDLE",
        0xC000_0017 => "NO_MEMORY",
        0xC000_001D => "ILLEGAL_INSTRUCTION",
        0xC000_0025 => "NONCONTINUABLE_EXCEPTION",
        0xC000_0026 => "INVALID_DISPOSITION",
        0xC000_008C => "ARRAY_BOUNDS_EXCEEDED",
        0xC000_008D => "FLT_DENORMAL_OPERAND",
        0xC000_008E => "FLT_DIVIDE_BY_ZERO",
        0xC000_008F => "FLT_INEXACT_RESULT",
        0xC000_0090 => "FLT_INVALID_OPERATION",
        0xC000_0091 => "FLT_OVERFLOW",
        0xC000_0092 => "FLT_STACK_CHECK",
        0xC000_0093 => "FLT_UNDERFLOW",
        0xC000_0094 => "INT_DIVIDE_BY_ZERO",
        0xC000_0095 => "INT_OVERFLOW",
        0xC000_0096 => "PRIV_INSTRUCTION",
        0xC000_00FD => "STACK_OVERFLOW",
        0xC000_0135 => "DLL_NOT_FOUND",
        0xC000_0142 => "DLL_INIT_FAILED",
        0xC000_013A => "CONTROL_C_EXIT",
        0xC000_0194 => "POSSIBLE_DEADLOCK",
        0xC000_0374 => "HEAP_CORRUPTION",
        0xC000_0409 => "STACK_BUFFER_OVERRUN",
        0xC000_0420 => "ASSERTION_FAILURE",
        0xC000_0602 => "FAIL_FAST_EXCEPTION",
        0x8000_0001 => "GUARD_PAGE_VIOLATION",
        0x8000_0002 => "DATATYPE_MISALIGNMENT",
        0x8000_0003 => "BREAKPOINT",
        0x8000_0004 => "SINGLE_STEP",
        0x4000_001F => "WX86_BREAKPOINT",
        0x406D_1388 => "SET_THREAD_NAME",
        0xE06D_7363 => "CPP_EH_EXCEPTION",
        0xE043_4352 => "CLR_EXCEPTION",
        0xE043_4F4D => "CLR_EXCEPTION",
        _ => return None,
    })
}

/// `1d 2h 3m 4s`, leaving out leading zero units.
fn uptime(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{s}s"),
        (0, 0, _) => format!("{m}m {s}s"),
        (0, _, _) => format!("{h}h {m}m {s}s"),
        _ => format!("{d}d {h}h {m}m {s}s"),
    }
}

fn hex(v: u64) -> String {
    format!("{v:#x}")
}

/// The last component of a Windows or Unix path.
fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Cuts dump-supplied text to [`MAX_TEXT`] characters and drops control characters.
fn clip(s: &str) -> String {
    let mut out: String = s.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect();
    if s.chars().filter(|c| !c.is_control()).nth(MAX_TEXT).is_some() {
        out.pop();
        out.push('\u{2026}');
    }
    out
}

#[cfg(test)]
#[path = "../tests/it/support/synth_minidump.rs"]
pub(crate) mod synth;

#[cfg(test)]
mod tests;
