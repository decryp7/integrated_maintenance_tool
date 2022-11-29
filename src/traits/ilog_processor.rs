use std::error::Error;
use std::path::Path;

pub trait ILogProcessor {
    fn process(&self, log_file_path: impl AsRef<Path>) -> Result<(), Box<dyn Error>>;
}