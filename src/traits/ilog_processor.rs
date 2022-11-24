use std::error::Error;

pub trait ILogProcessor {
    fn process(&self, log_file_path: &str) -> Result<(), Box<dyn Error>>;
}