use imt::config::app_config::AppConfig;
use imt::log_processors::dir_log_processor::DirLogProcessor;
use imt::log_processors::filter_log_processor::FilterLogProcessor;
use imt::log_processors::log_processor::LogProcessor;
use regex::bytes::Regex;
use std::env;
use std::error::Error;
use std::fs::metadata;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let target = match imt::cli::parse_args(&args) {
        Ok(path) => path,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(imt::cli::USAGE_EXIT_CODE);
        }
    };
    match run(target) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(target: &str) -> Result<(), Box<dyn Error>> {
    let app_config: AppConfig = confy::load("imt", "imt").map_err(|e| format!("Unable to load configuration: {e}"))?;

    let path = confy::get_configuration_file_path("imt", "imt")?;
    println!("Loading configuration from {}.", path.display());
    println!("Running on {:?} with {:?}.", target, app_config);

    let md = metadata(target).map_err(|e| format!("Unable to read {target}: {e}"))?;

    let log_processor: Box<dyn LogProcessor> = if md.is_dir() {
        Box::new(DirLogProcessor {
            log_file_ext: String::from(&*app_config.log_processor_config.log_file_ext),
            filter_log_processor: FilterLogProcessor {
                filter_regex: Regex::new(&app_config.filter_log_processor_config.filter_regex)?,
                line_start_regex: Regex::new(&app_config.log_processor_config.line_start_regex)?,
            },
        })
    } else {
        Box::new(FilterLogProcessor {
            filter_regex: Regex::new(&app_config.filter_log_processor_config.filter_regex)?,
            line_start_regex: Regex::new(&app_config.log_processor_config.line_start_regex)?,
        })
    };

    match log_processor.process(target.to_string()) {
        Ok(_) => println!("Processing OK!"),
        Err(e) => println!("Unable to process file: {:?}", e),
    }

    Ok(())
}
