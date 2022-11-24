use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::io::BufRead;

pub struct LogProcessor {
    
}

impl LogProcessor {
    pub fn process(&self, log_file_path: &str) -> Result<(), Box<dyn Error>>{
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