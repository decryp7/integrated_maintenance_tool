//! Windows adapter for [`DotnetSource`]: the classic `.NET CLR *` counters through PDH and
//! runtime detection from the process's module list (Toolhelp). Decisions live in `dotnet`.
//!
//! Counters are added with `PdhAddEnglishCounterW`, so English names work on every locale.
//! Counters that cannot be added are reported and skipped; without `.NET CLR Memory\Process ID`
//! instances cannot be mapped to PIDs, so no counter is read (runtime detection still works).

use crate::dotnet::{classify_modules, disambiguate, ClrCounter, ClrValue, DotnetSource, Kind, Opened, Runtime};
use crate::pdh::{array, check, raw_array};
use anyhow::Result;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhOpenQueryW, PDH_HCOUNTER, PDH_HQUERY, PDH_NO_DATA,
};

struct ClrPdh {
    query: Option<PDH_HQUERY>,
    counters: Vec<(PDH_HCOUNTER, ClrCounter)>,
}

// SAFETY: PDH handles are process-wide, not tied to the opening thread; `ClrPdh` is not
// `Sync`, so its `&mut self` methods never run concurrently.
unsafe impl Send for ClrPdh {}

impl Drop for ClrPdh {
    fn drop(&mut self) {
        if let Some(q) = self.query {
            // SAFETY: closes the query opened in `open`; its counters are closed with it.
            unsafe { PdhCloseQuery(q) };
        }
    }
}

fn add(query: PDH_HQUERY, c: ClrCounter) -> Result<PDH_HCOUNTER> {
    let path = c.wildcard_path();
    let mut handle = PDH_HCOUNTER::default();
    // SAFETY: `query` is open; the path string outlives the call.
    check(
        unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(path.as_str()), 0, &mut handle) },
        "PdhAddEnglishCounter",
        &path,
    )?;
    Ok(handle)
}

pub fn open() -> Result<Opened> {
    let mut query = PDH_HQUERY::default();
    // SAFETY: plain FFI call with a valid out pointer.
    check(unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) }, "PdhOpenQuery", "")?;
    let mut pdh = ClrPdh {
        query: Some(query),
        counters: Vec::new(),
    };
    let mut problems = Vec::new();
    match add(query, ClrCounter::ProcessId) {
        Ok(h) => {
            pdh.counters.push((h, ClrCounter::ProcessId));
            let mut missing = Vec::new();
            for c in ClrCounter::ALL.into_iter().skip(1) {
                match add(query, c) {
                    Ok(h) => pdh.counters.push((h, c)),
                    Err(_) => missing.push(c.wildcard_path()),
                }
            }
            if !missing.is_empty() {
                problems.push(format!(".NET CLR counters not available, skipped: {}", missing.join(", ")));
            }
            // Establishes the first sample.
            // SAFETY: `query` is open.
            unsafe { PdhCollectQueryData(query) };
        }
        Err(e) => problems.push(format!(
            ".NET Framework runtime counters unavailable ({e}); the '.NET CLR' performance counter categories may be missing or disabled"
        )),
    }
    Ok(Opened {
        source: Box::new(pdh),
        problems,
    })
}

impl DotnetSource for ClrPdh {
    fn collect(&mut self) -> Result<Vec<ClrValue>> {
        let Some(query) = self.query.filter(|_| !self.counters.is_empty()) else {
            return Ok(Vec::new());
        };
        // SAFETY: `query` is open.
        let status = unsafe { PdhCollectQueryData(query) };
        if status == PDH_NO_DATA {
            // No CLR instance exists right now.
            return Ok(Vec::new());
        }
        check(status, "PdhCollectQueryData", ".NET CLR counters")?;
        let mut out = Vec::new();
        for (handle, counter) in &self.counters {
            // SAFETY: handles were added to the collected query.
            let items: Vec<(String, f64)> = if counter.kind() == Kind::Rate {
                unsafe { raw_array(*handle) }.into_iter().map(|(n, v)| (n, v as f64)).collect()
            } else {
                unsafe { array(*handle) }
            };
            let (mut names, values): (Vec<String>, Vec<f64>) = items.into_iter().unzip();
            disambiguate(&mut names);
            out.extend(names.into_iter().zip(values).map(|(instance, value)| ClrValue {
                counter: *counter,
                instance,
                value,
            }));
        }
        Ok(out)
    }

    fn runtime(&mut self, pid: u32) -> Option<Runtime> {
        loaded_modules(pid).and_then(|m| classify_modules(m.iter().map(String::as_str)))
    }
}

/// File names of the modules loaded in `pid` (32- and 64-bit); `None` when unreadable.
fn loaded_modules(pid: u32) -> Option<Vec<String>> {
    // SAFETY: plain FFI call; the handle is closed below.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) }.ok()?;
    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut names = Vec::new();
    // SAFETY: `snap` is a valid snapshot and `entry` a correctly sized out struct.
    let mut ok = unsafe { Module32FirstW(snap, &mut entry) }.is_ok();
    while ok {
        let len = entry.szModule.iter().position(|c| *c == 0).unwrap_or(entry.szModule.len());
        names.push(String::from_utf16_lossy(&entry.szModule[..len]));
        // SAFETY: as above.
        ok = unsafe { Module32NextW(snap, &mut entry) }.is_ok();
    }
    // SAFETY: closes the snapshot handle created above.
    let _ = unsafe { CloseHandle(snap) };
    Some(names)
}
