//! `imt-plugin-host --plugin <id>`: serves one built-in plugin over `imt.plugin.v1` (IMT starts it when the
//! plugin runs isolated; see `imt_plugin_host::isolation`). `--list` prints the ids it can serve.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        for p in imt_plugin_host::builtins::all() {
            println!("{} {} {}", p.id(), p.version(), p.name());
        }
        return;
    }
    let id = args.iter().position(|a| a == "--plugin").and_then(|i| args.get(i + 1));
    let Some(plugin) = id.and_then(|id| imt_plugin_host::builtins::find(id)) else {
        eprintln!("usage: imt-plugin-host --plugin <id> | --list");
        std::process::exit(2);
    };
    let opts = imt_plugin_host::builtins::serve_options(plugin.id());
    if let Err(e) = imt_plugin_sdk::serve_with(plugin, opts) {
        eprintln!("imt-plugin-host: {e}");
        std::process::exit(1);
    }
}
