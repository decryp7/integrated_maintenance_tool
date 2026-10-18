//! Marker kinds for Windows event logs (`.evtx` imported by [`crate::EvtxImporter`]).
//!
//! Rules match the importer's fields: `Provider` (the `System/Provider/@Name`
//! attribute), `EventId` and, for annotators, `Computer`. Provider names are
//! the ones Windows writes in the `Name` attribute, which for classic sources
//! equals the event source shown in Event Viewer.
//!
//! Sources for the event ids (checked 2026-10):
//! * Application Error 1000 is the application crash event; Windows Error
//!   Reporting 1001 reports the crash to WER — Microsoft Learn, "Troubleshoot
//!   application or service crashing behavior":
//!   <https://learn.microsoft.com/en-us/troubleshoot/windows-server/performance/troubleshoot-application-service-crashing-behavior>
//! * .NET Runtime 1026 (process terminated by an unhandled exception) and
//!   Application Hang 1002 (program stopped responding): commonly paired with
//!   1000 in Microsoft Q&A threads
//!   (<https://learn.microsoft.com/en-us/answers/questions/55228/event-id-1000-and-1026>);
//!   no dedicated Microsoft Learn page was found.
//! * Service Control Manager 7034 "The %1 service terminated unexpectedly":
//!   <https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2008-r2-and-2008/dd349369(v=ws.10)>;
//!   7031 is the same crash with a recovery action configured (Geoff
//!   Chappell's SCM event reference). 7036 "entered the %2 state" drives the
//!   service span annotator.
//! * Kernel-Power 41: "the system has rebooted without cleanly shutting down
//!   first" —
//!   <https://learn.microsoft.com/en-us/previous-versions/orphan-topics/ws.10/ff943877(v=ws.10)>.
//! * EventLog 6005 (event log service started, at boot), 6006 (stopped, at a
//!   clean shutdown) and 6008 (the previous shutdown was unexpected): long
//!   standing since Windows NT 4.0 SP4; no current Microsoft Learn page was
//!   found, only secondary references.
//! * Kernel-General 12 (operating system started) and 13 (operating system is
//!   shutting down): secondary references only.
//! * BugCheck 1001 "The computer has rebooted from a bugcheck", written by
//!   provider `Microsoft-Windows-WER-SystemErrorReporting` (source BugCheck):
//!   <https://learn.microsoft.com/en-ca/answers/questions/5838090/event-1001-bugcheck>.
//! * disk 7 (bad block), 51 (error during a paging operation), 153 (I/O
//!   retried): 153 per Microsoft Learn "Troubleshoot data corruption and disk
//!   errors"
//!   (<https://learn.microsoft.com/en-us/troubleshoot/windows-server/backup-and-storage/troubleshoot-data-corruption-and-disk-errors>);
//!   7 and 51 from secondary references.
//! * WindowsUpdateClient 19 (installation successful) and 20 (installation
//!   failed):
//!   <https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2008-R2-and-2008/cc735642(v=ws.10)>;
//!   43 (installation started) from secondary references only.
//! * Security-Auditing 4625 "An account failed to log on":
//!   <https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/auditing/event-4625>.
//! * Process Monitor (`Result` of [`crate::ProcmonCsvImporter`] records):
//!   `ACCESS DENIED`, `SHARING VIOLATION` and `PRIVILEGE NOT HELD` are the NTSTATUS
//!   names ProcMon shows for permission, locking and privilege failures.

use imt_core::plugin::markers::{MarkerRole, PluginMarkerKind};

pub const APPLICATION_ERROR: &str = "Application Error";
pub const WER: &str = "Windows Error Reporting";
pub const DOTNET: &str = ".NET Runtime";
pub const APPLICATION_HANG: &str = "Application Hang";
pub const SCM: &str = "Service Control Manager";
pub const EVENTLOG: &str = "EventLog";
pub const KERNEL_POWER: &str = "Microsoft-Windows-Kernel-Power";
pub const KERNEL_GENERAL: &str = "Microsoft-Windows-Kernel-General";
pub const BUGCHECK: &str = "Microsoft-Windows-WER-SystemErrorReporting";
pub const DISK: &str = "disk";
pub const UPDATE: &str = "Microsoft-Windows-WindowsUpdateClient";
pub const SECURITY: &str = "Microsoft-Windows-Security-Auditing";
/// ProcMon results that mean "not allowed".
pub const PROCMON_DENIED: [&str; 3] = ["ACCESS DENIED", "SHARING VIOLATION", "PRIVILEGE NOT HELD"];

/// Kinds used only by annotators (no rules): their style for the spans.
pub const DOWNTIME: &str = "downtime";
pub const SERVICE_UP: &str = "service-up";

fn rule(kind: &str, label: &str, provider: &str, ids: &[&str]) -> PluginMarkerKind {
    let k = PluginMarkerKind::new(kind, label).when("Provider", "eq", provider);
    match ids {
        [one] => k.when("EventId", "eq", one),
        many => k.when_in("EventId", many),
    }
}

/// The Windows marker set.
pub fn kinds() -> Vec<PluginMarkerKind> {
    vec![
        rule("app-crash", "Application crash", APPLICATION_ERROR, &["1000"])
            .role(MarkerRole::Crash)
            .icon("x-circle")
            .color("#F2545B")
            .describe("Application Error 1000: a process crashed (faulting application and module in the event)."),
        rule("wer-report", "Error report", WER, &["1001"])
            .icon("file")
            .color("#C98BDB")
            .describe("Windows Error Reporting 1001: a crash or hang was reported to WER."),
        rule("dotnet-crash", ".NET unhandled exception", DOTNET, &["1026"])
            .role(MarkerRole::Crash)
            .badge("NET", "hexagon")
            .color("#F2545B")
            .describe(".NET Runtime 1026: a process was terminated by an unhandled exception."),
        rule("app-hang", "Application hang", APPLICATION_HANG, &["1002"])
            .icon("timer")
            .color("#E9AE3F")
            .describe("Application Hang 1002: a program stopped responding and was closed."),
        rule("service-crash", "Service crashed", SCM, &["7031", "7034"])
            .role(MarkerRole::Crash)
            .icon("server")
            .color("#F2545B")
            .describe("Service Control Manager 7031/7034: a service terminated unexpectedly."),
        rule("unexpected-shutdown", "Unexpected shutdown", EVENTLOG, &["6008"])
            .role(MarkerRole::Crash)
            .icon("power")
            .color("#FF4FA3")
            .describe("EventLog 6008, logged at boot: the previous shutdown was unexpected."),
        rule("kernel-power", "Dirty reboot", KERNEL_POWER, &["41"])
            .role(MarkerRole::Crash)
            .icon("zap")
            .color("#FF4FA3")
            .describe("Kernel-Power 41: rebooted without cleanly shutting down (crash, hang or power loss)."),
        rule("bugcheck", "Blue screen", BUGCHECK, &["1001"])
            .role(MarkerRole::Crash)
            .badge("BSD", "diamond")
            .color("#FF4FA3")
            .describe("BugCheck 1001: the computer rebooted from a bugcheck (stop code and dump in the event)."),
        rule("eventlog-started", "Event log started (boot)", EVENTLOG, &["6005"])
            .role(MarkerRole::Start)
            .icon("power")
            .color("#3FBF7F")
            .describe("EventLog 6005: the event log service started, i.e. Windows booted."),
        rule("eventlog-stopped", "Event log stopped (shutdown)", EVENTLOG, &["6006"])
            .role(MarkerRole::Stop)
            .icon("power")
            .color("#A7B0C2")
            .describe("EventLog 6006: the event log service stopped during a clean shutdown."),
        rule("os-started", "OS started", KERNEL_GENERAL, &["12"])
            .role(MarkerRole::Start)
            .icon("rocket")
            .color("#3FBF7F")
            .describe("Kernel-General 12: the operating system started."),
        rule("os-shutdown", "OS shutting down", KERNEL_GENERAL, &["13"])
            .role(MarkerRole::Stop)
            .icon("restart")
            .color("#A7B0C2")
            .describe("Kernel-General 13: the operating system is shutting down."),
        rule("disk-error", "Disk error", DISK, &["7", "51", "153"])
            .icon("disk")
            .color("#E9AE3F")
            .describe("disk 7 (bad block), 51 (paging I/O error) or 153 (I/O retried): check the disk and its path."),
        rule("update-installed", "Update installed", UPDATE, &["19"])
            .icon("download")
            .color("#3987E5")
            .describe("WindowsUpdateClient 19: an update was installed successfully."),
        rule("update-failed", "Update failed", UPDATE, &["20"])
            .icon("download")
            .color("#F2545B")
            .describe("WindowsUpdateClient 20: an update failed to install (error code in the event)."),
        rule("update-started", "Update install started", UPDATE, &["43"])
            .icon("download")
            .color("#A7B0C2")
            .describe("WindowsUpdateClient 43: installation of an update started."),
        rule("logon-failure", "Logon failure", SECURITY, &["4625"])
            .icon("lock")
            .color("#E9AE3F")
            .describe("Security 4625: an account failed to log on (see Status/SubStatus and LogonType)."),
        PluginMarkerKind::new("access-denied", "Access denied")
            .when_in("Result", &PROCMON_DENIED)
            .icon("lock")
            .color("#F2545B")
            .describe("Process Monitor: an operation was denied (ACCESS DENIED, SHARING VIOLATION or PRIVILEGE NOT HELD)."),
        PluginMarkerKind::new(DOWNTIME, "Downtime")
            .icon("power")
            .color("#FF4FA3")
            .describe("From the last event before a shutdown to the next boot, on one computer."),
        PluginMarkerKind::new(SERVICE_UP, "Service running")
            .icon("server")
            .color("#3FBF7F")
            .describe("A service between entering the running state and stopping or crashing (SCM 7036/7031/7034)."),
    ]
}

#[cfg(test)]
mod tests;
