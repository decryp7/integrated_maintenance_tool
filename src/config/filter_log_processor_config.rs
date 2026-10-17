use serde_derive::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
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
