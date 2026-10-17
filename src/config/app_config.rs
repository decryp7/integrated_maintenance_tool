use crate::config::filter_log_processor_config::FilterLogProcessorConfig;
use crate::config::log_processor_config::LogProcessorConfig;
use serde_derive::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct AppConfig {
    pub log_processor_config: LogProcessorConfig,
    pub filter_log_processor_config: FilterLogProcessorConfig,
}
