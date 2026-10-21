//! `imt-plugins`: serves IMT's built-in plugins, each in its own process (ADR 0017).
//!
//! - `imt-plugins --plugin <id>`: serves one plugin until IMT closes the connection (IMT starts this; the
//!   bootstrap comes on stdin, or from `--imt-bootstrap <file>` for the plugin's elevated process);
//! - `imt-plugins --manifests`: prints every built-in's manifest as a JSON array and exits (IMT reads them once
//!   at start; no plugin code beyond registration runs).

use imt_plugin_api::{Contributions, Plugin};
use std::sync::Arc;

/// Every built-in plugin. Add new plugins here.
fn builtins() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(imt_plugin_registry::RegistryPlugin),
        Arc::new(imt_plugin_otel::OtelPlugin),
        Arc::new(imt_plugin_syslog::SyslogPlugin),
        Arc::new(imt_plugin_windows::WindowsPlugin),
        Arc::new(imt_plugin_web::WebPlugin),
        Arc::new(imt_plugin_formats::FormatsPlugin),
        Arc::new(imt_plugin_profiling::ProfilingPlugin),
        Arc::new(imt_plugin_tools::ToolsPlugin::new()),
        Arc::new(imt_plugin_native::NativePlugin::new()),
        // Captures are shared by every window (one capture list in the plugin's process).
        Arc::new(imt_plugin_dotnet::DotnetPlugin::default()),
    ]
}

fn manifests() -> Result<String, String> {
    let mut out = Vec::new();
    for p in builtins() {
        let mut c = Contributions::default();
        p.register(&mut c).map_err(|e| format!("plugin \"{}\": {e:#}", p.id()))?;
        let m = c.manifest(p.as_ref());
        m.check()?;
        out.push(m);
    }
    serde_json::to_string(&out).map_err(|e| e.to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--manifests") {
        match manifests() {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("imt-plugins: {e}");
                std::process::exit(2);
            }
        }
        return;
    }
    let Some(id) = args.iter().position(|a| a == "--plugin").and_then(|i| args.get(i + 1)) else {
        eprintln!("usage: imt-plugins --plugin <id> | --manifests");
        std::process::exit(2);
    };
    let Some(plugin) = builtins().into_iter().find(|p| p.id() == id) else {
        eprintln!("imt-plugins: no built-in plugin \"{id}\"");
        std::process::exit(2);
    };
    if let Err(e) = imt_plugin_api::serve(plugin) {
        eprintln!("imt-plugins: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_builtin_registers_and_describes_itself() {
        let json = super::manifests().unwrap();
        let list: Vec<imt_plugin_api::Manifest> = serde_json::from_str(&json).unwrap();
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_ref()).collect();
        assert_eq!(ids.len(), 10, "{ids:?}");
        assert!(ids.contains(&"net.decryptology.tools") && ids.contains(&"net.decryptology.profiling"));
        assert!(ids.contains(&"net.decryptology.native"));
        assert!(ids.contains(&"net.decryptology.dotnet"));
    }
}
