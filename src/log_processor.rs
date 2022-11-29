use std::error::Error;
use std::fs::File;
use std::io::{BufReader, LineWriter, Write};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use crate::traits::ilog_processor::ILogProcessor;
use regex::Regex;

pub struct LogProcessor {
    pub filter_regex: Regex,
    pub line_start_regex: Regex,
}

impl ILogProcessor for LogProcessor {
    fn process(&self, log_file_path: impl AsRef<Path>) -> Result<(), Box<dyn Error>>{
        let log_file = match File::open(&log_file_path) {
            Ok(file) => file,
            Err(e) => return Err(Box::new(e)),
        };

        let file_name = log_file_path.as_ref().file_stem().expect("Unable to get file name");
        let directory = log_file_path.as_ref().parent().expect("Unable to get directory");
        let file_extension = log_file_path.as_ref().extension().expect("Unable to get file extension");
        let mut output_file_path = PathBuf::from(directory);
        output_file_path.push(format!("{}{}{}",file_name.to_str().unwrap(), "-filtered.", file_extension.to_str().unwrap()));

        let filtered_log_file = match File::create(&output_file_path) {
            Ok(file) => file,
            Err(e) => return Err(Box::new(e)),
        };

        let mut filtered_log_file_writer = LineWriter::new(filtered_log_file);
        let mut log_line = String::new();

        for line in BufReader::new(log_file).lines() {
            match line {
                Ok(l) =>{
                    let is_log_line = self.line_start_regex.is_match(&l);
                    if is_log_line {
                        if !log_line.is_empty() {
                            if self.filter_regex.is_match(&log_line) {
                                filtered_log_file_writer.write_all(log_line.as_bytes()).expect("Unable to write to filtered log file");
                                filtered_log_file_writer.write_all(b"\n")?;
                            }
                            log_line.clear();
                        }
                    }
                    log_line.push_str(&l);
                },
                Err(e) => return Err(Box::new(e)),
            }
        }
        Ok(())
    }
}