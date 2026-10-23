//! systemd unit generation (Linux). Install with:
//! `debrief-agent service print-unit --config /etc/debrief-agent.toml | sudo tee /etc/systemd/system/debrief-agent.service`
//! then `sudo systemctl daemon-reload && sudo systemctl enable --now debrief-agent`.

use super::{single_line, DESCRIPTION};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct UnitOptions {
    pub exe: PathBuf,
    pub config: PathBuf,
    /// Run as this user (default: root).
    pub user: Option<String>,
}

/// systemd quoting: double quotes, with `\` and `"` escaped and `%` doubled.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))
}

/// The unit file text. The agent needs no network, so the unit denies it.
pub fn unit_file(o: &UnitOptions) -> Result<String, String> {
    let exe = single_line(&o.exe)?;
    let config = single_line(&o.config)?;
    let user = match &o.user {
        Some(u) if u.is_empty() || !u.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) => {
            return Err(format!("\"{u}\" is not a valid user name"))
        }
        Some(u) => format!("User={u}\n"),
        None => String::new(),
    };
    Ok(format!(
        "[Unit]
Description={DESCRIPTION}
After=local-fs.target

[Service]
Type=simple
ExecStart={} run --config {}
{user}Restart=on-failure
RestartSec=10
TimeoutStopSec=90
KillSignal=SIGTERM
NoNewPrivileges=yes
PrivateNetwork=yes
ProtectSystem=full

[Install]
WantedBy=multi-user.target
",
        quote(&exe),
        quote(&config)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot() {
        let unit = unit_file(&UnitOptions {
            exe: "/usr/local/bin/debrief-agent".into(),
            config: "/etc/debrief agent/100%.toml".into(),
            user: Some("debrief".into()),
        })
        .unwrap();
        assert_eq!(
            unit,
            r#"[Unit]
Description=Debrief agent: scheduled and triggered diagnostics collection (local files only)
After=local-fs.target

[Service]
Type=simple
ExecStart="/usr/local/bin/debrief-agent" run --config "/etc/debrief agent/100%%.toml"
User=debrief
Restart=on-failure
RestartSec=10
TimeoutStopSec=90
KillSignal=SIGTERM
NoNewPrivileges=yes
PrivateNetwork=yes
ProtectSystem=full

[Install]
WantedBy=multi-user.target
"#
        );
    }

    #[test]
    fn refuses_injection() {
        let base = UnitOptions {
            exe: "/bin/debrief-agent".into(),
            config: "/etc/x.toml".into(),
            user: None,
        };
        assert!(!unit_file(&base).unwrap().contains("User="));
        assert!(unit_file(&UnitOptions {
            config: "/etc/x.toml\nExecStartPre=/bin/evil".into(),
            ..base.clone()
        })
        .is_err());
        assert!(unit_file(&UnitOptions {
            exe: "/bin/\nx".into(),
            ..base.clone()
        })
        .is_err());
        assert!(unit_file(&UnitOptions {
            user: Some("root\nGroup=x".into()),
            ..base.clone()
        })
        .is_err());
        assert!(unit_file(&UnitOptions {
            user: Some(String::new()),
            ..base
        })
        .is_err());
        assert_eq!(quote(r#"a\b"c"#), r#""a\\b\"c""#);
    }
}
