//! Windows Performance Data Helper binding (PDH). Compiled on Windows only and not
//! covered by tests on other platforms; all decisions live in `counters` (pure code).
//!
//! Counters are added with `PdhAddEnglishCounterW` so English paths work on any locale.
//! Wildcard instances (`\Process(*)\...`) are read with `PdhGetFormattedCounterArrayW`,
//! single instances with `PdhGetFormattedCounterValue`. When a `Process` counter is
//! requested, `\Process(*)\ID Process` is added so instance names map to PIDs.

use crate::counters::{CounterPath, CounterSource, CounterValue};
use anyhow::{bail, Result};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
};

pub struct Pdh {
    query: PDH_HQUERY,
    counters: Vec<(PDH_HCOUNTER, CounterPath)>,
}

fn check(status: u32, what: &str, path: &str) -> Result<()> {
    if status != 0 {
        bail!("{what} failed for {path} (PDH status 0x{status:08X})");
    }
    Ok(())
}

impl Pdh {
    pub fn open(paths: &[CounterPath]) -> Result<Pdh> {
        let mut query = PDH_HQUERY::default();
        // SAFETY: plain FFI call with a valid out pointer.
        check(unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) }, "PdhOpenQuery", "")?;
        let mut pdh = Pdh {
            query,
            counters: Vec::new(),
        };
        let mut all = paths.to_vec();
        let wants_pid =
            paths.iter().any(|p| p.is_process()) && !paths.iter().any(|p| p.is_process() && p.counter.eq_ignore_ascii_case("ID Process"));
        if wants_pid {
            all.push(CounterPath::parse("\\Process(*)\\ID Process")?);
        }
        for p in all {
            let text = p.to_string();
            let mut handle = PDH_HCOUNTER::default();
            // SAFETY: `query` is open; the path string outlives the call.
            check(
                unsafe { PdhAddEnglishCounterW(pdh.query, &HSTRING::from(text.as_str()), 0, &mut handle) },
                "PdhAddEnglishCounter",
                &text,
            )?;
            pdh.counters.push((handle, p));
        }
        // Rate counters need a first collection as baseline.
        // SAFETY: `query` is open.
        unsafe { PdhCollectQueryData(pdh.query) };
        Ok(pdh)
    }
}

// SAFETY: PDH query and counter handles are process-wide handles, not tied to the thread that
// opened them; `Pdh` is not `Sync`, so its `&mut self` methods never run concurrently.
unsafe impl Send for Pdh {}

impl Drop for Pdh {
    fn drop(&mut self) {
        // SAFETY: closes the query opened in `open`; counters are closed with it.
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// SAFETY: `counter` must belong to an open, collected query.
unsafe fn single(counter: PDH_HCOUNTER) -> Option<f64> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status = PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut value);
    // Every bit pattern is a valid f64, so reading the union field eagerly is sound.
    (status == 0 && value.CStatus == 0).then_some(value.Anonymous.doubleValue)
}

/// SAFETY: `counter` must belong to an open, collected query.
unsafe fn array(counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
    let (mut size, mut count) = (0u32, 0u32);
    if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None) != PDH_MORE_DATA || size == 0 {
        return Vec::new();
    }
    // u64 elements keep the buffer 8-byte aligned for the item structs.
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
    if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
        return Vec::new();
    }
    (0..count as usize)
        .filter_map(|i| {
            let item = &*items.add(i);
            (item.FmtValue.CStatus == 0).then(|| Some((item.szName.to_string().ok()?, item.FmtValue.Anonymous.doubleValue)))?
        })
        .collect()
}

impl CounterSource for Pdh {
    fn collect(&mut self) -> Result<Vec<CounterValue>> {
        // SAFETY: `query` is open.
        check(unsafe { PdhCollectQueryData(self.query) }, "PdhCollectQueryData", "")?;
        let mut out = Vec::new();
        for (handle, path) in &self.counters {
            // SAFETY: handles were added to the collected query.
            if path.is_wildcard() {
                out.extend(unsafe { array(*handle) }.into_iter().map(|(inst, value)| CounterValue {
                    path: path.with_instance(&inst),
                    value,
                }));
            } else if let Some(value) = unsafe { single(*handle) } {
                out.push(CounterValue { path: path.clone(), value });
            }
        }
        Ok(out)
    }
}
