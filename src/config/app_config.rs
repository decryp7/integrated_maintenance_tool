use serde_derive::{Serialize, Deserialize};
use crate::config::filter_log_processor_config::FilterLogProcessorConfig;
use crate::config::log_processor_config::LogProcessorConfig;

#[derive(Serialize, Deserialize)]
pub struct AppConfig {
    pub log_processor_config: LogProcessorConfig,
    pub filter_log_processor_config: FilterLogProcessorConfig,
}

impl Default for AppConfig  {
    fn default() -> Self {
        Self {
            log_processor_config: LogProcessorConfig::default(),
            filter_log_processor_config: FilterLogProcessorConfig::default(),
        }
    }
}