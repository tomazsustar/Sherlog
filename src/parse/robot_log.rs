// c:\work\git\Sherlog\src\parse\robot_log.rs

use crate::model::{LogEntry, LogLevel, LogSource, LogSourceContents};
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Check if a file is a Robot Framework debug.txt log by validating first 3 timestamp lines
pub fn is_robot_log(mut reader: impl std::io::Read + std::io::Seek) -> bool {
    let re = Regex::new(r"^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}\.\d{6}) - (\w+) - (.*)$")
        .expect("Invalid regex pattern");
    let separator_re = Regex::new(r"^[=\-~]+$").expect("Invalid separator regex");
    
    let bufreader = BufReader::new(&mut reader);
    let mut timestamp_lines = 0;
    
    for line_result in bufreader.lines() {
        if let Ok(line) = line_result {
            // Skip separator lines
            if separator_re.is_match(&line) {
                continue;
            }
            
            // Check if line matches Robot Framework format
            if re.is_match(&line) {
                timestamp_lines += 1;
                if timestamp_lines >= 3 {
                    // Reset reader position to beginning
                    let _ = reader.seek(std::io::SeekFrom::Start(0));
                    return true;
                }
            } else if !line.trim().is_empty() {
                // Found a non-empty, non-separator line that doesn't match format
                let _ = reader.seek(std::io::SeekFrom::Start(0));
                return false;
            }
        } else {
            break;
        }
    }
    
    // Reset reader position to beginning
    let _ = reader.seek(std::io::SeekFrom::Start(0));
    
    // If we found at least 3 valid timestamp lines, it's a Robot log
    timestamp_lines >= 3
}

/// Parse Robot Framework debug.txt log files
/// 
/// Format: YYYY-MM-DD HH:MM:SS.microseconds - LEVEL - message
/// Example: 2025-12-18 22:50:36.585690 - INFO - Selecting tracker 10.62.33.92
pub fn to_log_entries(reader: impl std::io::Read, name: String) -> Result<LogSource, std::io::Error> {
    let reader = BufReader::new(reader);
    
    // Regex pattern: YYYY-MM-DD HH:MM:SS.microseconds - LEVEL - message
    let re = Regex::new(r"^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}\.\d{6}) - (\w+) - (.*)$")
        .expect("Invalid regex pattern");
    
    // Pattern for separator lines (===== or -----)
    let separator_re = Regex::new(r"^[=\-~]+$").expect("Invalid separator regex");
    
    let mut entries = Vec::new();
    let mut current_entry: Option<LogEntry> = None;
    let mut last_timestamp: Option<DateTime<Utc>> = None;
    let mut pending_separators: Vec<String> = Vec::new();
    
    for line_result in reader.lines() {
        let line = line_result?;
        
        if let Some(caps) = re.captures(&line) {
            // Save previous entry if it exists
            if let Some(entry) = current_entry.take() {
                entries.push(entry);
            }
            
            // Parse new entry
            let date = &caps[1];
            let time = &caps[2];
            let level_str = &caps[3];
            let message = &caps[4];
            
            // Parse timestamp
            let datetime_str = format!("{} {}", date, time);
            let timestamp = match NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S%.6f") {
                Ok(naive_dt) => DateTime::<Utc>::from_utc(naive_dt, Utc),
                Err(_) => {
                    log::warn!("Failed to parse timestamp: {}", datetime_str);
                    continue;
                }
            };
            
            // Add any pending separators with the appropriate timestamp
            if !pending_separators.is_empty() {
                let sep_timestamp = last_timestamp.unwrap_or(timestamp);
                for sep in pending_separators.drain(..) {
                    entries.push(LogEntry {
                        timestamp: sep_timestamp,
                        severity: LogLevel::Info,
                        message: sep,
                        ..Default::default()
                    });
                }
            }
            
            last_timestamp = Some(timestamp);
            
            // Parse log level
            let severity = match level_str.to_uppercase().as_str() {
                "TRACE" => LogLevel::Trace,
                "DEBUG" => LogLevel::Debug,
                "INFO" => LogLevel::Info,
                "WARN" => LogLevel::Warning,
                "ERROR" | "FAIL" => LogLevel::Error,
                _ => {
                    log::warn!("Unknown log level: {}", level_str);
                    LogLevel::Info
                }
            };
            
            current_entry = Some(LogEntry {
                timestamp,
                severity,
                message: message.to_string(),
                ..Default::default()
            });
        } else if separator_re.is_match(&line) {
            // Always add separators to pending list to be processed before next log entry
            pending_separators.push(line);
        } else if let Some(ref mut entry) = current_entry {
            // Multi-line message continuation
            entry.message.push('\n');
            entry.message.push_str(&line);
        }
        // else: skip other lines before first timestamp
    }
    
    // Don't forget the last entry
    if let Some(entry) = current_entry {
        entries.push(entry);
    }
    
    // Add any remaining separators at the end with last timestamp
    if !pending_separators.is_empty() {
        if let Some(ts) = last_timestamp {
            for sep in pending_separators.drain(..) {
                entries.push(LogEntry {
                    timestamp: ts,
                    severity: LogLevel::Info,
                    message: sep,
                    ..Default::default()
                });
            }
        }
    }
    
    Ok(LogSource {
        name,
        children: LogSourceContents::Entries(entries),
    })
}

pub fn from_file(path: &PathBuf) -> Result<LogSource, std::io::Error> {
    let file = File::open(path)?;
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .to_string();
    to_log_entries(file, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_robot_log_line() {
        let line = "2025-12-18 22:50:36.585690 - INFO - Selecting tracker 10.62.33.92";
        let re = Regex::new(r"^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}\.\d{6}) - (\w+) - (.*)$").unwrap();
        
        let caps = re.captures(line).expect("Should match");
        assert_eq!(&caps[1], "2025-12-18");
        assert_eq!(&caps[2], "22:50:36.585690");
        assert_eq!(&caps[3], "INFO");
        assert_eq!(&caps[4], "Selecting tracker 10.62.33.92");
    }
}