//! launchd property list generation (macOS). Install with:
//! `imt-agent service print-plist --config /etc/imt-agent.toml | sudo tee /Library/LaunchDaemons/net.decryptology.imt-agent.plist`
//! then `sudo launchctl bootstrap system /Library/LaunchDaemons/net.decryptology.imt-agent.plist`.

use super::{single_line, LAUNCHD_LABEL};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct PlistOptions {
    pub exe: PathBuf,
    pub config: PathBuf,
    /// Where launchd writes the agent's stdout/stderr (startup errors).
    pub log_dir: PathBuf,
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The plist text: started at boot, restarted when it exits with an error.
pub fn plist(o: &PlistOptions) -> Result<String, String> {
    let exe = xml(&single_line(&o.exe)?);
    let config = xml(&single_line(&o.config)?);
    // A macOS document: joined with '/' whatever the platform printing it (not the host's separator).
    let log_dir = single_line(&o.log_dir)?;
    let log = |name: &str| xml(&format!("{}/{name}", log_dir.trim_end_matches('/')));
    let (out, err) = (log("launchd.out.log"), log("launchd.err.log"));
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>run</string>
    <string>--config</string>
    <string>{config}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>ExitTimeOut</key>
  <integer>90</integer>
  <key>StandardOutPath</key>
  <string>{out}</string>
  <key>StandardErrorPath</key>
  <string>{err}</string>
</dict>
</plist>
"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot() {
        let text = plist(&PlistOptions {
            exe: "/usr/local/bin/imt-agent".into(),
            config: "/etc/imt&agent.toml".into(),
            log_dir: "/var/log/imt-agent".into(),
        })
        .unwrap();
        assert_eq!(
            text,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>net.decryptology.imt-agent</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/imt-agent</string>
    <string>run</string>
    <string>--config</string>
    <string>/etc/imt&amp;agent.toml</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>ExitTimeOut</key>
  <integer>90</integer>
  <key>StandardOutPath</key>
  <string>/var/log/imt-agent/launchd.out.log</string>
  <key>StandardErrorPath</key>
  <string>/var/log/imt-agent/launchd.err.log</string>
</dict>
</plist>
"#
        );
    }

    #[test]
    fn log_paths_use_slashes_on_every_platform() {
        // Regression: on Windows the log file names were joined with '\'.
        let text = plist(&PlistOptions {
            exe: "/x".into(),
            config: "/c".into(),
            log_dir: "/var/log/a&b/".into(),
        })
        .unwrap();
        assert!(text.contains("<string>/var/log/a&amp;b/launchd.out.log</string>"), "{text}");
        assert!(text.contains("<string>/var/log/a&amp;b/launchd.err.log</string>"), "{text}");
    }

    #[test]
    fn escapes_and_refuses_line_breaks() {
        assert_eq!(xml("<a & \"b\">"), "&lt;a &amp; &quot;b&quot;&gt;");
        let ok = PlistOptions {
            exe: "/x".into(),
            config: "/c".into(),
            log_dir: "/l".into(),
        };
        assert!(plist(&PlistOptions {
            config: "/c\n</string>".into(),
            ..ok.clone()
        })
        .is_err());
        assert!(plist(&PlistOptions {
            exe: "/x\n".into(),
            ..ok.clone()
        })
        .is_err());
        assert!(plist(&PlistOptions {
            log_dir: "/l\n".into(),
            ..ok
        })
        .is_err());
    }
}
