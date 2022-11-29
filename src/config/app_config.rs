use serde_derive::{Serialize, Deserialize};
use crate::config::log_processor_config::LogProcessorConfig;

#[derive(Serialize, Deserialize)]
pub struct AppConfig {
    pub log_processor_config : LogProcessorConfig,
}

impl Default for AppConfig  {
    fn default() -> Self {
        Self {
            log_processor_config: LogProcessorConfig::default(),
        }
    }
}