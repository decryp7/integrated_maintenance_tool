use imt::log_processor::LogProcessor;
use std::env;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err("Missing argument!")?;
    }

    let log_processor = LogProcessor {};
    match log_processor.process(&args[1]) {
        Ok(_) => println!("Processing OK!"),
        Err(e) => println!("Unable to process file: {:?}", e),
    }
    Ok(())
}
