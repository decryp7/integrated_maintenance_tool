use serde_derive::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct LogProcessorConfig {
    pub line_start_regex: String,
}

impl Default for LogProcessorConfig {
    fn default() -> Self {
        Self {
            line_start_regex: r"^\d{4}/\d{2}/\d{2}".to_string(),
        }
    }
}