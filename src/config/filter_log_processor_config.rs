use serde_derive::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct FilterLogProcessorConfig {
    pub filter_regex: String,
}

impl Default for FilterLogProcessorConfig {
    fn default() -> Self {
        Self {
            filter_regex: r"^.*$".to_string(),
        }
    }
}