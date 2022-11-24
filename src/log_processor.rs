use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::io::BufRead;
use crate::traits::ilog_processor::ILogProcessor;

pub struct LogProcessor {
    
}

impl ILogProcessor for LogProcessor {
    fn process(&self, log_file_path: &str) -> Result<(), Box<dyn Error>>{
        let log_file = match File::open(log_file_path) {
            Ok(file) => file,
            Err(e) => return Err(Box::new(e)),
        };

        for line in BufReader::new(log_file).lines() {
            match line {
                Ok(l) => println!("{}", l),
                Err(e) => return Err(Box::new(e)),
            }
        }
        Ok(())
    }
}