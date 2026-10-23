//! Standalone collector: prints the path of the `.debriefz` bundle it wrote.
//! Exit code 0 on success, 2 for bad arguments, 1 for fatal errors.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err((code, msg)) = debrief_collect::cli::run(&args, &mut std::io::stdout()) {
        eprintln!("debrief-collect: {msg}");
        std::process::exit(code);
    }
}
