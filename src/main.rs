use std::env;
use std::error::Error;
use std::fs::metadata;
use regex::Regex;
use imt::config::app_config::AppConfig;
use imt::log_processors::dir_log_processor::DirLogProcessor;
use imt::log_processors::filter_log_processor::FilterLogProcessor;
use imt::log_processors::log_processor::LogProcessor;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err("Missing argument!")?;
    }

    let app_config: AppConfig = confy::load("imt", "imt").expect("Unable to load configuration.");

    let md = metadata(&args[1]).expect("Unable to get metadata");
    let log_processor: Box<dyn LogProcessor>;

    if md.is_dir(){
        log_processor = Box::new(DirLogProcessor {
            log_file_ext: String::from(&*app_config.log_processor_config.log_file_ext),
            filter_log_processor: FilterLogProcessor {
                filter_regex: Regex::new(&*app_config.filter_log_processor_config.filter_regex)?,
                line_start_regex: Regex::new(&*app_config.log_processor_config.line_start_regex)?,
            }
        });
    }else{
        log_processor = Box::new(FilterLogProcessor {
            filter_regex: Regex::new(&*app_config.filter_log_processor_config.filter_regex)?,
            line_start_regex: Regex::new(&*app_config.log_processor_config.line_start_regex)?,
        });
    }

    match log_processor.process((&*args[1]).to_string()) {
        Ok(_) => println!("Processing OK!"),
        Err(e) => println!("Unable to process file: {:?}", e),
    }

    Ok(())
}
