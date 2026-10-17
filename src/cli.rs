//! Command-line arguments of the `imt` tool.

/// Exit code for a command-line usage error (as in BSD `sysexits.h` / common CLI practice).
pub const USAGE_EXIT_CODE: u8 = 2;

/// How to call the tool, printed when the arguments are wrong.
pub fn usage(program: &str) -> String {
    format!("Usage: {program} <path>\n\nFilters the log file, or every log file in the folder, at <path>.")
}

/// The path to process from `args` (the program name first), or the usage text.
pub fn parse_args(args: &[String]) -> Result<&str, String> {
    let program = args.first().map(String::as_str).unwrap_or("imt");
    match args.get(1).map(String::as_str) {
        Some(path) if !path.trim().is_empty() && args.len() == 2 => Ok(path),
        _ => Err(usage(program)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn takes_exactly_one_path() {
        assert_eq!(parse_args(&args(&["imt", "logs"])), Ok("logs"));
        for bad in [&["imt"][..], &["imt", ""], &["imt", "a", "b"], &[]] {
            let e = parse_args(&args(bad)).unwrap_err();
            assert!(e.starts_with("Usage: imt <path>"), "{e}");
        }
        assert!(parse_args(&args(&["./imt-cli"]))
            .unwrap_err()
            .starts_with("Usage: ./imt-cli <path>"));
    }
}
