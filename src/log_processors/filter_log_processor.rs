use crate::log_processors::log_processor::LogProcessor;
use regex::bytes::Regex;
use std::error::Error;
use std::fs::File;
use std::io::BufRead;
use std::io::{BufReader, LineWriter, Write};
use std::path::{Path, PathBuf};

pub struct FilterLogProcessor {
    pub filter_regex: Regex,
    pub line_start_regex: Regex,
}

impl LogProcessor for FilterLogProcessor {
    fn process(&self, log_file_path: String) -> Result<(), Box<dyn Error>> {
        let log_file = match File::open(&log_file_path) {
            Ok(file) => file,
            Err(e) => return Err(Box::new(e)),
        };

        let file_path = Path::new(&log_file_path);
        let file_name = file_path.file_stem().expect("Unable to get file name");
        let directory = file_path.parent().expect("Unable to get directory");
        let file_extension = file_path.extension().expect("Unable to get file extension");
        let mut output_file_path = PathBuf::from(directory);
        output_file_path.push(format!(
            "{}{}{}",
            file_name.to_str().unwrap(),
            "-filtered.",
            file_extension.to_str().unwrap()
        ));

        let filtered_log_file = match File::create(&output_file_path) {
            Ok(file) => file,
            Err(e) => return Err(Box::new(e)),
        };

        let mut filtered_log_file_writer = LineWriter::new(filtered_log_file);
        //let mut log_line = String::new();

        let mut reader = BufReader::new(log_file);
        let mut buf = vec![];

        while let Ok(_) = reader.read_until(b'\n', &mut buf) {
            if buf.is_empty() {
                break;
            }
            //let line = String::from_utf8_lossy(&buf);
            let is_log_line = self.line_start_regex.is_match(&buf);
            if is_log_line {
                if self.filter_regex.is_match(&buf) {
                    filtered_log_file_writer
                        .write_all(&buf)
                        .expect("Unable to write to filtered log file");
                    println!("{}", String::from_utf8_lossy(&buf));
                }
            }
            buf.clear();
        }

        Ok(())
    }
}
