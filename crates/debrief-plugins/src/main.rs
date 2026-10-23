//! `debrief-plugins`: serves Debrief's built-in plugins, each in its own process (ADR 0017).
//!
//! - `debrief-plugins --plugin <id>`: serves one plugin until Debrief closes the connection (Debrief starts this; the
//!   bootstrap comes on stdin, or from `--debrief-bootstrap <file>` for the plugin's elevated process);
//! - `debrief-plugins --manifests`: prints every built-in's manifest as a JSON array and exits (Debrief reads them once
//!   at start; no plugin code beyond registration runs).

use debrief_plugin_api::{Contributions, Plugin};
use std::sync::Arc;

/// Every built-in plugin. Add new plugins here.
fn builtins() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(debrief_plugin_registry::RegistryPlugin),
        Arc::new(debrief_plugin_otel::OtelPlugin),
        Arc::new(debrief_plugin_syslog::SyslogPlugin),
        Arc::new(debrief_plugin_windows::WindowsPlugin),
        Arc::new(debrief_plugin_web::WebPlugin),
        Arc::new(debrief_plugin_formats::FormatsPlugin),
        Arc::new(debrief_plugin_profiling::ProfilingPlugin),
        Arc::new(debrief_plugin_tools::ToolsPlugin::new()),
        Arc::new(debrief_plugin_native::NativePlugin::new()),
        // Captures are shared by every window (one capture list in the plugin's process).
        Arc::new(debrief_plugin_dotnet::DotnetPlugin::default()),
        Arc::new(debrief_plugin_dumps::DumpsPlugin::new()),
        Arc::new(debrief_plugin_dumps::DumpCapturePlugin),
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

/// mimalloc v3, tuned (see `debrief-alloc`): plugins parse files per event like the engine.
#[global_allocator]
static GLOBAL: debrief_alloc::MiMalloc = debrief_alloc::MiMalloc;

fn main() {
    debrief_alloc::tune();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--manifests") {
        match manifests() {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("debrief-plugins: {e}");
                std::process::exit(2);
            }
        }
        return;
    }
    let Some(id) = args.iter().position(|a| a == "--plugin").and_then(|i| args.get(i + 1)) else {
        eprintln!("usage: debrief-plugins --plugin <id> | --manifests");
        std::process::exit(2);
    };
    let Some(plugin) = builtins().into_iter().find(|p| p.id() == id) else {
        eprintln!("debrief-plugins: no built-in plugin \"{id}\"");
        std::process::exit(2);
    };
    if let Err(e) = debrief_plugin_api::serve(plugin) {
        eprintln!("debrief-plugins: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_builtin_registers_and_describes_itself() {
        let json = super::manifests().unwrap();
        let list: Vec<debrief_plugin_api::Manifest> = serde_json::from_str(&json).unwrap();
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_ref()).collect();
        assert_eq!(ids.len(), 12, "{ids:?}");
        assert!(ids.contains(&"net.decryptology.tools") && ids.contains(&"net.decryptology.profiling"));
        assert!(ids.contains(&"net.decryptology.native"));
        assert!(ids.contains(&"net.decryptology.dotnet"));
        assert!(ids.contains(&"net.decryptology.dumps") && ids.contains(&"net.decryptology.dumps.capture"));
    }
}
