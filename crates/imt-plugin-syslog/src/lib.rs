//! IMT plugin that imports syslog and systemd journal exports.
//!
//! * `syslog` importer: RFC 5424 (`<PRI>1 TIMESTAMP HOST APP PROCID MSGID [SD] MSG`,
//!   structured data as `sd.<id>.<param>` fields) and RFC 3164 / BSD lines
//!   (`<PRI>Oct 14 10:00:00 host app[pid]: msg`, with or without `<PRI>`, as in
//!   `/var/log/syslog`) plus the rsyslog ISO-timestamp variant. Lines that do not
//!   start a new entry are appended to the previous message.
//! * `journald` importer: `journalctl -o json` (one object per line).
//!
//! Records carry `Facility` (name), `Severity` (name), `Host`, `App`, `ProcId`
//! and `MsgId` where the format provides them. Severity 0-2 maps to Fatal,
//! 3 Error, 4 Warning, 5-6 Information and 7 Debug.
//!
//! It runs in its own process (`imt-plugins --plugin <id>`, ADR 0017).

mod journald;
mod syslog;

pub use journald::JournaldImporter;
pub use syslog::SyslogImporter;

use anyhow::Result;
use imt_plugin_api::model::Level;
use imt_plugin_api::{Contributions, Plugin};

pub struct SyslogPlugin;

impl Plugin for SyslogPlugin {
    fn id(&self) -> &'static str {
        "net.decryptology.syslog"
    }

    fn name(&self) -> &'static str {
        "Syslog & journald"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_importer(JournaldImporter);
        c.add_importer(SyslogImporter);
        Ok(())
    }
}

const FACILITIES: [&str; 24] = [
    "kern", "user", "mail", "daemon", "auth", "syslog", "lpr", "news", "uucp", "cron", "authpriv", "ftp", "ntp", "audit", "alert", "clock",
    "local0", "local1", "local2", "local3", "local4", "local5", "local6", "local7",
];

const SEVERITIES: [&str; 8] = ["emerg", "alert", "crit", "err", "warning", "notice", "info", "debug"];

fn facility_name(facility: u8) -> Option<&'static str> {
    FACILITIES.get(facility as usize).copied()
}

fn severity_name(severity: u8) -> &'static str {
    SEVERITIES[(severity & 7) as usize]
}

fn severity_level(severity: u8) -> Level {
    match severity {
        0..=2 => Level::Fatal,
        3 => Level::Error,
        4 => Level::Warning,
        5 | 6 => Level::Information,
        _ => Level::Debug,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_pri_parts() {
        assert_eq!(facility_name(4), Some("auth"));
        assert_eq!(facility_name(23), Some("local7"));
        assert_eq!(facility_name(24), None);
        assert_eq!(severity_name(3), "err");
        let levels: Vec<_> = (0..8).map(severity_level).collect();
        use Level::*;
        assert_eq!(levels, [Fatal, Fatal, Fatal, Error, Warning, Information, Information, Debug]);
    }

    #[test]
    fn plugin_registers_both_importers() {
        let mut c = Contributions::default();
        SyslogPlugin.register(&mut c).unwrap();
        let info = c.manifest(&SyslogPlugin);
        assert_eq!(&*info.id, "net.decryptology.syslog");
        assert_eq!(
            info.contributions(),
            vec!["importer:journald".to_string(), "importer:syslog".to_string()]
        );
        assert_eq!(SyslogPlugin.name(), "Syslog & journald");
        assert_eq!(SyslogPlugin.version(), env!("CARGO_PKG_VERSION"));
    }
}
