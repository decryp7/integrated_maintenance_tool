use serde_derive::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct LogProcessorConfig {
    pub filter_regex: String,
    pub line_start_regex: String,
}

impl Default for LogProcessorConfig  {
    fn default() -> Self {
        Self {
            filter_regex: r"^.*$".to_string(),
            line_start_regex: r"^\d{4}/\d{2}/\d{2}".to_string(),
        }
    }
}