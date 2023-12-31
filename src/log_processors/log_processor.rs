use std::error::Error;

pub trait LogProcessor {
    fn process(&self, log_file_path: String) -> Result<(), Box<dyn Error>>;
}
