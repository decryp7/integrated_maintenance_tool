//! Data Collector Set templates for Performance Monitor.
//!
//! [`presets`] lists curated counter groups (CPU, memory, disk, network) with a line of
//! guidance per counter. A [`CollectorTemplate`] combines presets, target processes and
//! logging options and renders a Data Collector Set XML that `logman import -n <name> -xml <file>`
//! and PerfMon's "Create from template" accept. Everything here is pure; running `logman`
//! is [`crate::perfmon_control`]'s job.
//!
//! Element names follow the documented `IDataCollectorSet` / `IPerformanceCounterDataCollector`
//! XML. Value encodings that could not be verified against Microsoft documentation are listed
//! in the comments next to the constants below.

use anyhow::{bail, Result};
use quick_xml::events::{BytesDecl, BytesText, Event};
use quick_xml::Writer;

/// Placeholder replaced by each target process instance (or `*`).
pub const PROCESS_PLACEHOLDER: &str = "{process}";
/// Added to every template: lets IMT map `Process(name)` rows to process ids.
pub const ID_PROCESS_COUNTER: &str = "\\Process({process})\\ID Process";
pub const DEFAULT_OUTPUT_DIR: &str = "%systemdrive%\\PerfLogs\\IMT\\";

// UNVERIFIED encodings (taken from exported templates and PLA enums as remembered):
// SubdirectoryFormat is a flag set (1 = pattern, 2 = computer name; 3 = both), LogFileFormat
// is plaCommaSeparated 0 / plaTabSeparated 1 / plaSql 2 / plaBinary 3, booleans are 0 / -1.
const SUBDIRECTORY_FORMAT: &str = "3";
const SUBDIRECTORY_PATTERN: &str = "yyyyMMdd\\-NNNNNN";
const TRUE: &str = "-1";
const FALSE: &str = "0";

#[derive(Debug, Clone, PartialEq)]
pub struct CounterPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Counter paths; `{process}` stands for the process instance.
    pub counters: Vec<String>,
}

/// One line on what a problem looks like for a counter path, for the UI.
pub fn guidance(counter: &str) -> Option<&'static str> {
    GUIDANCE.iter().find(|(c, _)| *c == counter).map(|(_, g)| *g)
}

const P: &str = "\\Process({process})\\";
macro_rules! g {
    ($($path:literal => $text:literal),* $(,)?) => { &[$(($path, $text)),*] };
}

const GUIDANCE: &[(&str, &str)] = g![
    "\\Process({process})\\% Processor Time" => "Sustained near 100% per core (or above 100% when multi-threaded) means the process is CPU bound.",
    "\\Process({process})\\% User Time" => "High share of the total means time is spent in application code, not the kernel.",
    "\\Process({process})\\% Privileged Time" => "A large share (over about 30%) points at kernel work: I/O, drivers, antivirus or heavy locking.",
    "\\Process({process})\\Thread Count" => "A steady climb signals a thread leak; very high counts suggest runaway thread creation.",
    "\\Processor(_Total)\\% Processor Time" => "Over 80% for minutes means the machine is saturated.",
    "\\Processor(_Total)\\% Privileged Time" => "Over 30% of busy time in kernel mode suggests driver, I/O or interrupt trouble.",
    "\\System\\Processor Queue Length" => "More than 2 per core for long stretches means threads wait for a CPU.",
    "\\System\\Context Switches/sec" => "Tens of thousands per core per second indicate thrashing or lock contention.",
    "\\Process({process})\\Private Bytes" => "A line that only rises, never returning after load, is a memory leak.",
    "\\Process({process})\\Working Set" => "Growing without bound or collapsing under memory pressure (trimming) both need a look.",
    "\\Process({process})\\Working Set - Private" => "Memory only this process holds in RAM; steady growth is a leak.",
    "\\Process({process})\\Virtual Bytes" => "Approaching the address-space limit (about 2 GB on 32-bit) leads to out-of-memory errors.",
    "\\Process({process})\\Page Faults/sec" => "Sustained high values mean the process is paging; check Memory\\Pages/sec.",
    "\\Process({process})\\Handle Count" => "Continuous growth means leaked handles; thousands more than normal is a bug.",
    "\\Process({process})\\Pool Paged Bytes" => "Growth over time points at a kernel paged-pool leak caused by this process.",
    "\\Process({process})\\Pool Nonpaged Bytes" => "Growth over time points at a nonpaged-pool leak caused by this process.",
    "\\Memory\\Available MBytes" => "Under about 10% of RAM, or under a few hundred MB on servers, means memory pressure.",
    "\\Memory\\% Committed Bytes In Use" => "Over 80-90% means the system is close to its commit limit.",
    "\\Memory\\Committed Bytes" => "Nearing the commit limit makes allocations fail; compare with RAM plus page file.",
    "\\Memory\\Pages/sec" => "Sustained over about 100 with high disk latency means hard faults and memory starvation.",
    "\\Memory\\Pool Nonpaged Bytes" => "Steady growth indicates a driver leaking nonpaged pool, risking a system hang.",
    "\\Process({process})\\IO Data Bytes/sec" => "Shows how much I/O the process generates; spikes should match its activity.",
    "\\Process({process})\\IO Read Bytes/sec" => "Unexpected sustained reads can reveal scans, missing caches or logging abuse.",
    "\\Process({process})\\IO Write Bytes/sec" => "Unexpected sustained writes can reveal runaway logging or temp file churn.",
    "\\PhysicalDisk(_Total)\\Avg. Disk sec/Read" => "Above 20 ms (SSD: 5 ms) sustained is a slow disk.",
    "\\PhysicalDisk(_Total)\\Avg. Disk sec/Write" => "Above 20 ms (SSD: 5 ms) sustained is a slow disk.",
    "\\PhysicalDisk(_Total)\\Current Disk Queue Length" => "Consistently more than 2 per spindle means requests pile up.",
    "\\PhysicalDisk(_Total)\\% Idle Time" => "Near 0% means the disk is saturated.",
    "\\Network Interface(*)\\Bytes Total/sec" => "Approaching the link speed (1 Gbit is about 125 MB/s) means the NIC is saturated.",
    "\\Network Interface(*)\\Output Queue Length" => "Persistently above 2 means the adapter cannot send fast enough.",
    "\\TCPv4\\Connections Established" => "A steady climb suggests connection leaks; a sudden drop suggests an outage.",
    "\\TCPv4\\Segments Retransmitted/sec" => "More than about 1% of segments sent indicates packet loss or congestion.",
    "\\Process({process})\\ID Process" => "Maps each Process instance to its process id, so restarts and duplicates can be told apart.",
];

fn preset(id: &str, name: &str, description: &str, counters: &[&str]) -> CounterPreset {
    CounterPreset {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        counters: counters.iter().map(|c| c.replace("\\Process\\", P)).collect(),
    }
}

/// The built-in presets: `cpu`, `memory`, `disk`, `network`.
pub fn presets() -> Vec<CounterPreset> {
    vec![
        preset(
            "cpu",
            "CPU",
            "Per-process and machine CPU use, run queue and context switches.",
            &[
                "\\Process\\% Processor Time",
                "\\Process\\% User Time",
                "\\Process\\% Privileged Time",
                "\\Process\\Thread Count",
                "\\Processor(_Total)\\% Processor Time",
                "\\Processor(_Total)\\% Privileged Time",
                "\\System\\Processor Queue Length",
                "\\System\\Context Switches/sec",
            ],
        ),
        preset(
            "memory",
            "Memory",
            "Per-process memory, handles and pools plus machine memory pressure; finds leaks.",
            &[
                "\\Process\\Private Bytes",
                "\\Process\\Working Set",
                "\\Process\\Working Set - Private",
                "\\Process\\Virtual Bytes",
                "\\Process\\Page Faults/sec",
                "\\Process\\Handle Count",
                "\\Process\\Pool Paged Bytes",
                "\\Process\\Pool Nonpaged Bytes",
                "\\Memory\\Available MBytes",
                "\\Memory\\% Committed Bytes In Use",
                "\\Memory\\Committed Bytes",
                "\\Memory\\Pages/sec",
                "\\Memory\\Pool Nonpaged Bytes",
            ],
        ),
        preset(
            "disk",
            "Disk",
            "Per-process I/O volume and physical disk latency and queueing.",
            &[
                "\\Process\\IO Data Bytes/sec",
                "\\Process\\IO Read Bytes/sec",
                "\\Process\\IO Write Bytes/sec",
                "\\PhysicalDisk(_Total)\\Avg. Disk sec/Read",
                "\\PhysicalDisk(_Total)\\Avg. Disk sec/Write",
                "\\PhysicalDisk(_Total)\\Current Disk Queue Length",
                "\\PhysicalDisk(_Total)\\% Idle Time",
            ],
        ),
        preset(
            "network",
            "Network",
            "Adapter throughput and queueing, TCP connections and retransmits.",
            &[
                "\\Network Interface(*)\\Bytes Total/sec",
                "\\Network Interface(*)\\Output Queue Length",
                "\\TCPv4\\Connections Established",
                "\\TCPv4\\Segments Retransmitted/sec",
            ],
        ),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Binary `.blg` (required for circular logs).
    Blg,
    Csv,
    Tsv,
}

impl LogFormat {
    fn code(self) -> &'static str {
        match self {
            LogFormat::Csv => "0",
            LogFormat::Tsv => "1",
            LogFormat::Blg => "3",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectorTemplate {
    pub name: String,
    /// Process instance names (without `.exe`); empty = all processes (`*`).
    pub processes: Vec<String>,
    /// Preset ids from [`presets`].
    pub presets: Vec<String>,
    /// Further counter paths, optionally with `{process}`.
    pub extra_counters: Vec<String>,
    pub sample_interval_secs: u32,
    pub format: LogFormat,
    pub output_dir: String,
    /// Circular log size in MB (BLG only).
    pub max_size_mb: Option<u32>,
    /// Stop after this many seconds.
    pub duration_secs: Option<u32>,
}

impl Default for CollectorTemplate {
    fn default() -> Self {
        CollectorTemplate {
            name: "IMT".into(),
            processes: Vec::new(),
            presets: vec!["cpu".into(), "memory".into()],
            extra_counters: Vec::new(),
            sample_interval_secs: 15,
            format: LogFormat::Csv,
            output_dir: DEFAULT_OUTPUT_DIR.into(),
            max_size_mb: None,
            duration_secs: None,
        }
    }
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// Instance names as PerfMon shows them: no `.exe`.
fn instance_name(p: &str) -> &str {
    let p = p.trim();
    match p.len().checked_sub(4) {
        Some(cut) if p.is_char_boundary(cut) && p[cut..].eq_ignore_ascii_case(".exe") => &p[..cut],
        _ => p,
    }
}

impl CollectorTemplate {
    pub fn validate(&self) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() || name.len() > 100 || name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)) {
            bail!("the collector name must be 1-100 characters without \\ / : * ? \" < > | or control characters");
        }
        if !(1..=86_400).contains(&self.sample_interval_secs) {
            bail!("the sample interval must be between 1 and 86400 seconds");
        }
        if self.max_size_mb == Some(0) || self.duration_secs == Some(0) {
            bail!("the maximum size and the duration must be at least 1 when set");
        }
        if self.max_size_mb.is_some() && self.format != LogFormat::Blg {
            bail!("a circular log (maximum size) needs the binary BLG format");
        }
        if self.output_dir.trim().is_empty() || has_control(&self.output_dir) {
            bail!("the output folder must not be empty or contain control characters");
        }
        for p in &self.processes {
            let n = instance_name(p);
            if n.is_empty() || has_control(n) || n.chars().any(|c| matches!(c, '\\' | '(' | ')' | '*' | '/')) {
                bail!("invalid process name \"{p}\": use the instance name such as w3wp or OrderService#1");
            }
        }
        let known = presets();
        for id in &self.presets {
            if !known.iter().any(|k| &k.id == id) {
                bail!(
                    "unknown preset \"{id}\" (available: {})",
                    known.iter().map(|k| k.id.as_str()).collect::<Vec<_>>().join(", ")
                );
            }
        }
        for c in &self.extra_counters {
            if !c.trim().starts_with('\\') || has_control(c) || c.trim().len() < 4 {
                bail!("invalid counter \"{c}\": expected a path like \\Object(Instance)\\Counter");
            }
        }
        if self.presets.is_empty() && self.extra_counters.is_empty() {
            bail!("select at least one preset or counter");
        }
        Ok(())
    }

    /// Every counter path, `{process}` expanded, duplicates removed, `ID Process` last.
    pub fn counters(&self) -> Result<Vec<String>> {
        self.validate()?;
        let known = presets();
        let mut templates: Vec<&str> = Vec::new();
        for id in &self.presets {
            if let Some(p) = known.iter().find(|k| &k.id == id) {
                templates.extend(p.counters.iter().map(String::as_str));
            }
        }
        templates.extend(self.extra_counters.iter().map(|c| c.trim()));
        templates.push(ID_PROCESS_COUNTER);
        let instances: Vec<&str> = if self.processes.is_empty() {
            vec!["*"]
        } else {
            self.processes.iter().map(|p| instance_name(p)).collect()
        };
        let mut out: Vec<String> = Vec::new();
        for t in templates {
            let expanded: Vec<String> = if t.contains(PROCESS_PLACEHOLDER) {
                instances.iter().map(|i| t.replace(PROCESS_PLACEHOLDER, i)).collect()
            } else {
                vec![t.to_string()]
            };
            for c in expanded {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        Ok(out)
    }

    /// The Data Collector Set XML (UTF-8 text with an XML declaration).
    pub fn to_xml(&self) -> Result<String> {
        let counters = self.counters()?;
        let name = self.name.trim();
        let mut w = Writer::new_with_indent(Vec::new(), b'\t', 1);
        w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
        let described = if self.processes.is_empty() {
            "all processes".to_string()
        } else {
            self.processes.iter().map(|p| instance_name(p)).collect::<Vec<_>>().join(", ")
        };
        let description = format!(
            "IMT performance capture ({}) for {described}, every {} s",
            self.presets.join(", "),
            self.sample_interval_secs
        );
        let text = |w: &mut Writer<Vec<u8>>, tag: &str, value: &str| -> std::io::Result<()> {
            w.create_element(tag).write_text_content(BytesText::new(value))?;
            Ok(())
        };
        w.create_element("DataCollectorSet")
            .write_inner_content(|w| -> std::io::Result<()> {
                text(w, "Name", name)?;
                text(w, "DisplayName", name)?;
                text(w, "Description", &description)?;
                text(w, "RootPath", self.output_dir.trim())?;
                text(w, "SubdirectoryFormat", SUBDIRECTORY_FORMAT)?;
                text(w, "SubdirectoryFormatPattern", SUBDIRECTORY_PATTERN)?;
                text(w, "Segment", FALSE)?;
                if let Some(mb) = self.max_size_mb {
                    text(w, "SegmentMaxSize", &mb.to_string())?;
                }
                if let Some(secs) = self.duration_secs {
                    text(w, "Duration", &secs.to_string())?;
                    text(w, "StopOnCompletion", TRUE)?;
                }
                w.create_element("PerformanceCounterDataCollector")
                    .write_inner_content(|w| -> std::io::Result<()> {
                        text(w, "Name", "PerformanceCounters")?;
                        text(w, "FileName", "PerfCounters")?;
                        text(w, "LogFileFormat", self.format.code())?;
                        text(w, "SampleInterval", &self.sample_interval_secs.to_string())?;
                        text(w, "LogAppend", FALSE)?;
                        text(w, "LogCircular", if self.max_size_mb.is_some() { TRUE } else { FALSE })?;
                        for c in &counters {
                            text(w, "Counter", c)?;
                            text(w, "CounterDisplayName", c)?;
                        }
                        Ok(())
                    })?;
                Ok(())
            })?;
        Ok(String::from_utf8(w.into_inner())?)
    }

    /// The `logman` command lines to import, start, stop and delete this set.
    pub fn logman_commands(&self, xml_path: &str) -> Vec<String> {
        let name = self.name.trim();
        vec![
            format!("logman import -n \"{name}\" -xml \"{}\"", xml_path.replace('"', "")),
            format!("logman start -n \"{name}\""),
            format!("logman stop -n \"{name}\""),
            format!("logman delete -n \"{name}\""),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_the_documented_four_and_fully_documented() {
        let all = presets();
        assert_eq!(
            all.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["cpu", "memory", "disk", "network"]
        );
        assert_eq!(all[1].counters.len(), 13);
        for c in all.iter().flat_map(|p| &p.counters).chain([&ID_PROCESS_COUNTER.to_string()]) {
            assert!(guidance(c).is_some_and(|g| g.ends_with('.')), "no guidance for {c}");
        }
        assert!(guidance("\\Nope\\Nothing").is_none());
        assert!(!format!("{all:?}").contains(".NET"));
    }

    #[test]
    fn default_template_validates_and_expands() {
        let t = CollectorTemplate::default();
        let c = t.counters().unwrap();
        assert!(c.contains(&"\\Process(*)\\Private Bytes".to_string()));
        assert_eq!(c.last().unwrap(), "\\Process(*)\\ID Process");
        assert_eq!(c.len(), 8 + 13 + 1);
    }

    #[test]
    fn expands_processes_and_dedupes() {
        let t = CollectorTemplate {
            processes: vec!["w3wp.EXE".into(), "OrderService#1".into(), "w3wp".into()],
            presets: vec!["memory".into(), "memory".into()],
            extra_counters: vec![
                "\\Memory\\Pages/sec".into(),
                " \\Process({process})\\Handle Count ".into(),
                "\\LogicalDisk(C:)\\Free Megabytes".into(),
            ],
            ..Default::default()
        };
        let c = t.counters().unwrap();
        assert!(c.contains(&"\\Process(w3wp)\\Handle Count".to_string()));
        assert!(c.contains(&"\\Process(OrderService#1)\\Private Bytes".to_string()));
        assert!(c.contains(&"\\LogicalDisk(C:)\\Free Megabytes".to_string()));
        assert_eq!(c.iter().filter(|x| x.ends_with("\\Handle Count")).count(), 2);
        assert_eq!(c.iter().filter(|x| x.as_str() == "\\Memory\\Pages/sec").count(), 1);
        assert_eq!(c.iter().filter(|x| x.as_str() == "\\Process(w3wp)\\ID Process").count(), 1);
    }

    #[test]
    fn validation_errors() {
        let ok = CollectorTemplate::default();
        let bad = |f: &dyn Fn(&mut CollectorTemplate)| {
            let mut t = ok.clone();
            f(&mut t);
            t.validate().unwrap_err().to_string()
        };
        assert!(bad(&|t| t.name = " ".into()).contains("collector name"));
        assert!(bad(&|t| t.name = "a/b".into()).contains("collector name"));
        assert!(bad(&|t| t.sample_interval_secs = 0).contains("interval"));
        assert!(bad(&|t| t.max_size_mb = Some(0)).contains("at least 1"));
        assert!(bad(&|t| t.duration_secs = Some(0)).contains("at least 1"));
        assert!(bad(&|t| t.max_size_mb = Some(100)).contains("BLG"));
        assert!(bad(&|t| t.output_dir = "".into()).contains("output folder"));
        assert!(bad(&|t| t.processes = vec!["a(b)".into()]).contains("invalid process"));
        assert!(bad(&|t| t.processes = vec![".exe".into()]).contains("invalid process"));
        assert!(bad(&|t| t.presets = vec!["dotnet-framework".into()]).contains("unknown preset"));
        assert!(bad(&|t| t.extra_counters = vec!["Memory".into()]).contains("invalid counter"));
        assert!(bad(&|t| t.presets = vec![]).contains("at least one"));
        assert!(CollectorTemplate {
            max_size_mb: Some(512),
            format: LogFormat::Blg,
            duration_secs: Some(60),
            ..ok
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn xml_shape_and_escaping() {
        let t = CollectorTemplate {
            name: "IMT leak & co".into(),
            processes: vec!["OrderService".into()],
            presets: vec!["cpu".into()],
            extra_counters: vec!["\\LogicalDisk(R&D)\\Free <MB>".into()],
            format: LogFormat::Blg,
            max_size_mb: Some(256),
            duration_secs: Some(3600),
            output_dir: "D:\\Perf & Logs\\".into(),
            sample_interval_secs: 5,
        };
        let xml = t.to_xml().unwrap();
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        for needle in [
            "<Name>IMT leak &amp; co</Name>",
            "<RootPath>D:\\Perf &amp; Logs\\</RootPath>",
            "<SubdirectoryFormat>3</SubdirectoryFormat>",
            "<SubdirectoryFormatPattern>yyyyMMdd\\-NNNNNN</SubdirectoryFormatPattern>",
            "<Segment>0</Segment>",
            "<SegmentMaxSize>256</SegmentMaxSize>",
            "<Duration>3600</Duration>",
            "<StopOnCompletion>-1</StopOnCompletion>",
            "<LogFileFormat>3</LogFileFormat>",
            "<SampleInterval>5</SampleInterval>",
            "<LogCircular>-1</LogCircular>",
            "<Counter>\\Process(OrderService)\\% Processor Time</Counter>",
            "<CounterDisplayName>\\LogicalDisk(R&amp;D)\\Free &lt;MB&gt;</CounterDisplayName>",
            "<Counter>\\Process(OrderService)\\ID Process</Counter>",
        ] {
            assert!(xml.contains(needle), "missing {needle} in\n{xml}");
        }
        // Well-formed: every start tag is closed.
        let mut reader = quick_xml::Reader::from_str(&xml);
        let mut depth = 0i32;
        loop {
            match reader.read_event().unwrap() {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth -= 1,
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(depth, 0);
        let plain = CollectorTemplate {
            format: LogFormat::Tsv,
            ..Default::default()
        }
        .to_xml()
        .unwrap();
        assert!(plain.contains("<LogFileFormat>1</LogFileFormat>") && plain.contains("<LogCircular>0</LogCircular>"));
        assert!(!plain.contains("SegmentMaxSize") && !plain.contains("<Duration>"));
        assert!(CollectorTemplate {
            format: LogFormat::Csv,
            ..Default::default()
        }
        .to_xml()
        .unwrap()
        .contains("<LogFileFormat>0</LogFileFormat>"));
        assert!(CollectorTemplate {
            name: "".into(),
            ..Default::default()
        }
        .to_xml()
        .is_err());
    }

    #[test]
    fn logman_commands_quote_the_name() {
        let cmds = CollectorTemplate::default().logman_commands("C:\\t\\\"x\".xml");
        assert_eq!(cmds[0], "logman import -n \"IMT\" -xml \"C:\\t\\x.xml\"");
        assert_eq!(
            cmds[1..],
            ["logman start -n \"IMT\"", "logman stop -n \"IMT\"", "logman delete -n \"IMT\""]
        );
    }
}
