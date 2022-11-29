use std::fs;
use std::fs::metadata;
use crate::log_processors::filter_log_processor::FilterLogProcessor;
use crate::log_processors::log_processor::LogProcessor;

pub struct DirLogProcessor{
    pub log_file_ext: String,
    pub filter_log_processor: FilterLogProcessor
}

impl LogProcessor for DirLogProcessor {
    fn process(&self, log_file_path: String) -> Result<(), String> {
        let md = metadata(&log_file_path).expect("Unable to get meta data of log file path");
        if !md.is_dir() {
            return Err("The log file path is not a valid directory!".into());
        }

        let paths = fs::read_dir(&log_file_path).expect("Unable to read path of directory");
        for path_result in paths {
            let path = path_result.expect("Unable to get path");
            if path.metadata().unwrap().is_file()
                && !path.path().into_os_string().into_string().unwrap().contains("filtered")
                && path.path().extension().unwrap().to_str().unwrap().to_ascii_lowercase() == self.log_file_ext.to_ascii_lowercase(){
                self.filter_log_processor.process(path.path().into_os_string().into_string().unwrap()).expect("Unable to filter log file.");
            }
        }

        Ok(())

    }
}