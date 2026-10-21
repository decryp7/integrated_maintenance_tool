use super::synth::*;
use super::*;
use imt_plugin_api::time::TimestampParser;
use std::path::Path;

const FIXTURE: &str = "tests/fixtures/crash.dmp";

fn import(bytes: &[u8]) -> Result<Imported> {
    let ts = TimestampParser::default();
    let ctx = ImportCtx::new(Path::new("crash.dmp"), &ts);
    MinidumpImporter.import(&ctx, bytes)
}

fn of_kind<'a>(records: &'a [Record], kind: &str) -> Vec<&'a Record> {
    records
        .iter()
        .filter(|r| r.fields.get("Kind").and_then(Value::as_str) == Some(kind))
        .collect()
}

fn field<'a>(r: &'a Record, name: &str) -> &'a Value {
    r.fields.get(name).unwrap_or_else(|| panic!("no field {name} in {:?}", r.fields))
}

#[test]
fn sniffs_the_signature() {
    let ts = TimestampParser::default();
    let ctx = ImportCtx::new(Path::new("x.bin"), &ts);
    assert_eq!(MinidumpImporter.id(), "minidump");
    assert_eq!(MinidumpImporter.sniff(&ctx, b"MDMP\x93\xa7"), 100);
    assert_eq!(MinidumpImporter.sniff(&ctx, b"ElfFile\0"), 0);
    assert_eq!(MinidumpImporter.sniff(&ctx, b""), 0);
    assert!(is_minidump(&sample_crash()));
}

#[test]
fn summarises_the_sample_crash() {
    let out = import(&sample_crash()).unwrap();
    assert_eq!(out.skipped, 0);
    let rs = &out.records;
    let ts = Some(i64::from(DUMP_TIME) * 1000);
    assert!(rs.iter().all(|r| r.ts == ts), "every record carries the dump time");
    assert_eq!(
        rs.iter().map(|r| r.line).collect::<Vec<_>>(),
        (1..=rs.len() as u32).collect::<Vec<_>>()
    );

    let sys = of_kind(rs, "system");
    assert_eq!(sys.len(), 1);
    assert_eq!(field(sys[0], "Os"), "Windows NT");
    assert_eq!(field(sys[0], "OsVersion"), "10.0.19045");
    assert_eq!(field(sys[0], "Cpu"), "amd64");
    assert_eq!(field(sys[0], "Processors"), 8);
    assert!(
        sys[0].message.starts_with("Windows NT 10.0.19045, amd64, 8 CPUs"),
        "{}",
        sys[0].message
    );
    assert!(!sys[0].fields.contains_key("ServicePack"));

    let proc = of_kind(rs, "process");
    assert_eq!(proc.len(), 1);
    assert_eq!(field(proc[0], "ProcessName"), "app.exe");
    assert_eq!(field(proc[0], "ProcessPath"), r"C:\Program Files\App\app.exe");
    assert_eq!(field(proc[0], "ProcessId"), 4242);
    assert_eq!(field(proc[0], "UptimeSeconds"), 3723);
    assert_eq!(field(proc[0], "ProcessCreateTime"), "2026-10-14T08:57:57+00:00");
    assert_eq!(proc[0].message, "Process app.exe (pid 4242), up 1h 2m 3s");

    let ex = of_kind(rs, "exception");
    assert_eq!(ex.len(), 1);
    let ex = ex[0];
    assert_eq!(ex.level, Level::Error);
    assert_eq!(field(ex, "ExceptionCode"), "0xC0000005");
    assert_eq!(field(ex, "ExceptionName"), "ACCESS_VIOLATION");
    assert_eq!(field(ex, "Reason"), "EXCEPTION_ACCESS_VIOLATION_READ");
    assert_eq!(field(ex, "Access"), "reading 0x0");
    assert_eq!(field(ex, "Module"), "app.exe");
    assert_eq!(field(ex, "Offset"), "0x1234");
    assert_eq!(field(ex, "ThreadId"), 7);
    assert_eq!(
        ex.message,
        "ACCESS_VIOLATION (0xC0000005) reading 0x0 at 0x7ff600001234 in app.exe+0x1234 on thread 7"
    );

    let frames = of_kind(rs, "frame");
    let labels: Vec<String> = frames
        .iter()
        .map(|f| format!("{}+{}", field(f, "Module").as_str().unwrap(), field(f, "Offset").as_str().unwrap()))
        .collect();
    assert_eq!(labels, ["app.exe+0x1234", "app.exe+0x2000", "kernel32.dll+0x17", "ntdll.dll+0x5aa"]);
    assert_eq!(field(frames[0], "Trust"), "context");
    assert!(frames[1..].iter().all(|f| field(f, "Trust") == "scan"));
    assert_eq!(field(frames[3], "Frame"), 3);
    assert_eq!(frames[0].message, "#0 app.exe+0x1234 (0x7ff600001234)");
    let trace = ex.exception.as_deref().unwrap();
    assert_eq!(trace.lines().count(), 4);
    assert!(trace.starts_with("#0 app.exe+0x1234"));

    let mods = of_kind(rs, "module");
    let names: Vec<&str> = mods.iter().map(|m| field(m, "Module").as_str().unwrap()).collect();
    assert_eq!(names, ["app.exe", "ntdll.dll", "kernel32.dll"]);
    assert_eq!(field(mods[0], "Version"), "1.2.3.4");
    assert_eq!(field(mods[0], "Base"), "0x7ff600000000");
    assert_eq!(field(mods[0], "Size"), 0x10000);
    assert_eq!(field(mods[1], "Path"), r"C:\Windows\System32\ntdll.dll");
    assert!(!mods[2].fields.contains_key("Version"));
    assert_eq!(mods[1].message, "Module ntdll.dll 10.0.19041.3636 at 0x7ffa00000000");
}

#[test]
fn committed_fixture_matches_the_builder() {
    // Regenerate with IMT_REGEN_FIXTURES=1 cargo test -p imt-plugin-windows minidump
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let bytes = sample_crash();
    if std::env::var_os("IMT_REGEN_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "{FIXTURE} is stale");
}

#[test]
fn rejects_files_that_are_not_dumps() {
    for bytes in [&b""[..], b"hello world", b"ElfFile\0rest", b"MDM"] {
        let err = import(bytes).map(|_| ()).unwrap_err().to_string();
        assert_eq!(err, "not a Windows minidump (missing MDMP signature)");
    }
}

#[test]
fn reports_truncated_and_wrong_version_headers() {
    let dump = sample_crash();
    let err = import(&dump[..20]).map(|_| ()).unwrap_err().to_string();
    assert!(err.starts_with("truncated minidump"), "{err}");

    let mut wrong = dump.clone();
    wrong[4..8].copy_from_slice(&0x1234u32.to_le_bytes());
    assert_eq!(import(&wrong).map(|_| ()).unwrap_err().to_string(), "unsupported minidump version");

    // Directory pointing past the end of the file.
    let mut lost = dump.clone();
    lost[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(import(&lost).map(|_| ()).unwrap_err().to_string().starts_with("truncated minidump"));
}

#[test]
fn every_truncation_is_handled() {
    let dump = sample_crash();
    for cut in 0..dump.len() {
        // Must not panic; cuts that keep the directory give a partial summary.
        let _ = import(&dump[..cut]);
    }
}

#[test]
fn damaged_streams_are_left_out() {
    // A dump with only a header and an empty directory.
    let out = import(&DumpBuilder::new().finish(0)).unwrap();
    assert_eq!(out.records.len(), 1);
    assert_eq!(out.records[0].ts, None, "no dump time");
    assert_eq!(out.records[0].message, "No exception: the dump was written on request");
    assert_eq!(out.records[0].level, Level::Information);

    // Garbage in every stream: the importer skips what it cannot read.
    let junk = [0xAB; 7];
    let mut b = DumpBuilder::new();
    for kind in [SYSTEM_INFO, MISC_INFO, EXCEPTION, THREAD_LIST, MODULE_LIST] {
        b.stream(kind, &junk);
    }
    let out = import(&b.finish(DUMP_TIME)).unwrap();
    assert_eq!(of_kind(&out.records, "exception").len(), 1);
    assert!(of_kind(&out.records, "system").is_empty());
}

#[test]
fn exception_without_system_info_has_no_frames() {
    let ctx = amd64_context(APP_BASE + 1, STACK_BASE);
    let dump = DumpBuilder::new()
        .module_list(&[(APP_BASE, 0x1000, "app.exe", None)])
        .exception(1, 0xC000_00FD, APP_BASE + 1, &[], &ctx)
        .finish(DUMP_TIME);
    let out = import(&dump).unwrap();
    let ex = of_kind(&out.records, "exception");
    assert_eq!(field(ex[0], "ExceptionName"), "STACK_OVERFLOW");
    assert_eq!(field(ex[0], "Module"), "app.exe");
    assert!(ex[0].exception.is_none());
    assert!(of_kind(&out.records, "frame").is_empty());
    // A process record from the module list alone.
    let proc = of_kind(&out.records, "process");
    assert_eq!(proc[0].message, "Process app.exe");
}

#[test]
fn falls_back_to_the_thread_context_and_unknown_codes() {
    let ctx = amd64_context(NTDLL_BASE + 0x10, STACK_BASE);
    let stack = stack64(&[APP_BASE + 0x40]);
    let dump = DumpBuilder::new()
        .system_info(ARCH_AMD64, (10, 0, 22631), 4, "Service Pack 1")
        .module_list(&[(APP_BASE, 0x1000, "/opt/app", None), (NTDLL_BASE, 0x1000, "ntdll.dll", None)])
        .thread_list(9, STACK_BASE, &stack, &ctx)
        // Truncated exception context: the thread's own context is used for the frames.
        .exception(9, 0xE000_0001, 0x1, &[], &[1, 2, 3])
        .finish(DUMP_TIME);
    let out = import(&dump).unwrap();
    let sys = of_kind(&out.records, "system");
    assert_eq!(field(sys[0], "ServicePack"), "Service Pack 1");
    assert!(sys[0].message.contains("(Service Pack 1)"));
    let ex = of_kind(&out.records, "exception")[0];
    assert_eq!(field(ex, "ExceptionCode"), "0xE0000001");
    // No friendly name: the reason from rust-minidump stands in.
    assert_eq!(field(ex, "ExceptionName"), field(ex, "Reason"));
    assert!(!ex.fields.contains_key("Module"), "0x1 lies in no module");
    let frames = of_kind(&out.records, "frame");
    assert_eq!(frames.len(), 2);
    assert_eq!(field(frames[0], "Module"), "ntdll.dll");
    assert_eq!(field(frames[1], "Module"), "app");
}

#[test]
fn scans_32_bit_stacks_and_stops_at_max_frames() {
    let base = 0x0040_0000u32;
    let esp = 0x0010_0000u32;
    let mut stack = Vec::new();
    for i in 0..(MAX_FRAMES as u32 + 10) {
        stack.extend_from_slice(&(base + 0x100 + i).to_le_bytes());
        stack.extend_from_slice(&7u32.to_le_bytes()); // non-code word between
    }
    let ctx = x86_context(0x1, esp);
    let dump = DumpBuilder::new()
        .system_info(ARCH_X86, (6, 1, 7601), 2, "")
        .module_list(&[(u64::from(base), 0x10000, "game.exe", Some((2, 0, 0, 0)))])
        .thread_list(3, u64::from(esp), &stack, &ctx)
        .exception(3, 0xC000_0005, 0xFFFF_FFFF_0000_0001, &[8, 0x1], &ctx)
        .finish(DUMP_TIME);
    let out = import(&dump).unwrap();
    let ex = of_kind(&out.records, "exception")[0];
    assert_eq!(field(ex, "Address"), "0x1", "sign-extended 32-bit address is masked");
    assert_eq!(field(ex, "Access"), "executing 0x1");
    let frames = of_kind(&out.records, "frame");
    assert_eq!(frames.len(), MAX_FRAMES);
    assert!(frames[0].fields.get("Module").is_none());
    assert_eq!(frames[0].message, "#0 0x1");
    assert_eq!(field(frames[1], "Offset"), "0x100");
    assert_eq!(field(frames[2], "Offset"), "0x101");
}

#[test]
fn stack_pointer_outside_the_stack_scans_nothing() {
    let ctx = amd64_context(APP_BASE, STACK_BASE + 0x1000);
    let dump = DumpBuilder::new()
        .system_info(ARCH_AMD64, (10, 0, 1), 1, "")
        .module_list(&[(APP_BASE, 0x1000, "app.exe", None)])
        .thread_list(1, STACK_BASE, &stack64(&[APP_BASE + 8]), &ctx)
        .exception(1, 0x8000_0003, APP_BASE, &[], &ctx)
        .finish(DUMP_TIME);
    let out = import(&dump).unwrap();
    let frames = of_kind(&out.records, "frame");
    assert_eq!(frames.len(), 1);
    let ex = of_kind(&out.records, "exception")[0];
    assert_eq!(field(ex, "ExceptionName"), "BREAKPOINT");
    assert!(!ex.fields.contains_key("Access"));
}

#[test]
fn caps_the_module_list() {
    let names: Vec<String> = (0..(MAX_MODULES as u64 + 3)).map(|i| format!("m{i}.dll")).collect();
    let refs: Vec<ModuleSpec> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (0x1000_0000 + i as u64 * 0x1000, 0x1000, n.as_str(), None))
        .collect();
    let out = import(&DumpBuilder::new().module_list(&refs).finish(DUMP_TIME)).unwrap();
    assert_eq!(of_kind(&out.records, "module").len(), MAX_MODULES);
    assert_eq!(out.skipped, 3);
}

#[test]
fn process_without_modules_uses_misc_info() {
    let out = import(&DumpBuilder::new().misc_info(99, 0).finish(0)).unwrap();
    let proc = of_kind(&out.records, "process");
    assert_eq!(proc[0].message, "Process unknown process (pid 99)");
    assert!(!proc[0].fields.contains_key("UptimeSeconds"));
    assert!(!proc[0].fields.contains_key("ProcessCreateTime"));
}

#[test]
fn pure_helpers() {
    assert_eq!(uptime(5), "5s");
    assert_eq!(uptime(65), "1m 5s");
    assert_eq!(uptime(3723), "1h 2m 3s");
    assert_eq!(uptime(90_061), "1d 1h 1m 1s");
    assert_eq!(file_name(r"C:\a\b.dll"), "b.dll");
    assert_eq!(file_name("/usr/lib/x.so"), "x.so");
    assert_eq!(file_name("plain"), "plain");
    assert_eq!(clip("a\u{0}b\nc"), "abc");
    let long = "x".repeat(MAX_TEXT + 5);
    let c = clip(&long);
    assert_eq!(c.chars().count(), MAX_TEXT);
    assert!(c.ends_with('\u{2026}'));
    assert_eq!(clip(&"y".repeat(MAX_TEXT)), "y".repeat(MAX_TEXT));
    assert_eq!(exception_name(0xC000_0409), Some("STACK_BUFFER_OVERRUN"));
    assert_eq!(exception_name(0xE06D_7363), Some("CPP_EH_EXCEPTION"));
    assert_eq!(exception_name(0x1234), None);
    // Every named code maps to a non-empty, upper-case name.
    for code in [0xC000_0005u32, 0x8000_0001, 0x4000_001F, 0x406D_1388, 0xE043_4F4D] {
        let n = exception_name(code).unwrap();
        assert!(n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'), "{n}");
    }
}

#[test]
fn scan_words_handles_unaligned_and_out_of_range_pointers() {
    let bytes: Vec<u8> = (0u8..32).collect();
    assert_eq!(scan_words(&bytes, 100, 99, 8).count(), 0, "sp below the region");
    assert_eq!(scan_words(&bytes, 100, 200, 8).count(), 0, "sp above the region");
    assert_eq!(scan_words(&bytes, 100, 100, 8).count(), 4);
    // Unaligned sp rounds up to the next word.
    let w: Vec<u64> = scan_words(&bytes, 100, 101, 8).collect();
    assert_eq!(w.len(), 3);
    assert_eq!(w[0], u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]));
    assert_eq!(
        scan_words(&bytes, 0, 4, 4).next(),
        Some(u64::from(u32::from_le_bytes([4, 5, 6, 7])))
    );
    let big = vec![0u8; MAX_SCAN_BYTES * 2];
    assert_eq!(scan_words(&big, 0, 0, 8).count(), MAX_SCAN_BYTES / 8);
}
