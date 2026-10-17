//! `imt-plugin-check <plugin executable> [plugin args...] [--grant <permission>]... [--fuzz <cases>] [--seed <n>]`
//!
//! Starts the plugin the way IMT does and runs the conformance kit against it. Exits 0 when every check
//! passes, 1 when one fails, 2 on a usage error. Permissions: `read-opened-files`, `network:<host>`.

use imt_plugin_sdk::conformance::{run_binary, Options};
use imt_plugin_sdk::{Permission, PermissionRequest};

fn usage() -> ! {
    eprintln!("usage: imt-plugin-check <plugin executable> [plugin args...] [--grant read-opened-files|network:<host>]... [--fuzz <cases>] [--seed <n>]");
    std::process::exit(2);
}

fn parse(args: Vec<String>) -> Result<(String, Vec<String>, Options), String> {
    let mut opts = Options::default();
    let mut exe = None;
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--grant" => {
                let g = it.next().ok_or("--grant needs a permission")?;
                let permission = match g.as_str() {
                    "read-opened-files" => Permission::ReadOpenedFiles,
                    other => match other.strip_prefix("network:") {
                        Some(host) => Permission::Network { host: host.into() },
                        None => return Err(format!("unknown permission \"{other}\"")),
                    },
                };
                opts.grant.push(PermissionRequest {
                    permission,
                    reason: String::new(),
                });
            }
            "--fuzz" => opts.fuzz_cases = it.next().and_then(|n| n.parse().ok()).ok_or("--fuzz needs a number")?,
            "--seed" => opts.seed = it.next().and_then(|n| n.parse().ok()).ok_or("--seed needs a number")?,
            "-h" | "--help" => return Err(String::new()),
            _ if exe.is_none() => exe = Some(a),
            _ => rest.push(a),
        }
    }
    Ok((exe.ok_or("name the plugin executable")?, rest, opts))
}

fn main() {
    let (exe, args, opts) = match parse(std::env::args().skip(1).collect()) {
        Ok(p) => p,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("imt-plugin-check: {e}");
            }
            usage()
        }
    };
    let report = run_binary(&exe, &args, opts);
    println!("{report}");
    std::process::exit(if report.passed() { 0 } else { 1 });
}
