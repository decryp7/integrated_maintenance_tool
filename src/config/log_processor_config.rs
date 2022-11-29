use serde_derive::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct LogProcessorConfig {
    pub log_file_ext: String,
    pub line_start_regex: String,
}

impl Default for LogProcessorConfig {
    fn default() -> Self {
        Self {
            log_file_ext: "log".to_string(),
            line_start_regex: r"^\d{4}/\d{2}/\d{2}".to_string(),
        }
    }
}