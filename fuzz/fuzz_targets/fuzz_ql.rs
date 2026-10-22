#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

use debrief_core::ql;

fuzz_target!(|data: &[u8]| {
    let Some((cur, rest)) = data.split_first_chunk::<2>() else { return };
    let text = String::from_utf8_lossy(rest);
    let fields = vec!["Cpu".to_string(), "MachineName".to_string()];
    let _ = ql::completions(&text, u16::from_le_bytes(*cur) as usize, &fields);
    if let Ok(program) = ql::parse(&text) {
        let mut r = debrief_core::model::Record::new(1);
        r.message = "timeout 42".into();
        r.fields.insert("Cpu".into(), 42.into());
        let recs = [r];
        let _ = ql::run(&program, &mut recs.iter().map(|r| (r, "src")));
    }
});
