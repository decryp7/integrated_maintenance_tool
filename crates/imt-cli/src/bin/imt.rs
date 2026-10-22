//! The command line; see `imt help` and docs/cli.md.

use imt_cli::{args, host::WorkerHost, output, run, Exit, BIN_NAME};
use std::io::Write;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let json = argv.iter().any(|a| a == "--json");
    let command = match args::parse(&argv) {
        Ok(c) => c,
        Err(message) => {
            if json {
                println!("{}", output::error_json(&message));
            }
            eprintln!("{BIN_NAME}: {message}\n\n{}", args::usage());
            std::process::exit(Exit::Usage.code());
        }
    };
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    if let Some(exit) = run::run_static(&command, &mut out) {
        std::process::exit(exit.code());
    }
    let exit = match WorkerHost::new() {
        Ok(host) => {
            let exit = run::run(command, &host, &mut out, &mut err);
            host.shutdown();
            exit
        }
        Err(e) => {
            let _ = writeln!(err, "{BIN_NAME}: {e}");
            if json {
                let _ = writeln!(out, "{}", output::error_json(&e.to_string()));
            }
            Exit::Failure
        }
    };
    let _ = out.flush();
    std::process::exit(exit.code());
}
