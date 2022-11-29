use std::env;
use std::error::Error;
use regex::Regex;
use imt::config::app_config::AppConfig;
use imt::log_processors::filter_log_processor::FilterLogProcessor;
use imt::log_processors::log_processor::LogProcessor;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err("Missing argument!")?;
    }

    let app_config: AppConfig = confy::load("imt", "imt").expect("Unable to load configuration.");

    let log_processor = FilterLogProcessor {
        filter_regex: Regex::new(&*app_config.log_processor_config.filter_regex)?,
        line_start_regex: Regex::new(&*app_config.log_processor_config.line_start_regex)?,
    };

    match log_processor.process(&args[1]) {
        Ok(_) => println!("Processing OK!"),
        Err(e) => println!("Unable to process file: {:?}", e),
    }

    Ok(())
}
