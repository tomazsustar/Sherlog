use super::super::model;
use super::glog;
use super::robot_log;
use super::sfile;

#[derive(Debug)]
pub enum LogParseError {
	IoError(std::io::Error),
	UnrecognizedFileExtension(std::ffi::OsString),
	NoFileExtension,
	UnrecognizedLogFile(std::path::PathBuf),
}

impl std::error::Error for LogParseError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			LogParseError::IoError(err) => Some(err),
			_ => None,
		}
	}
}

impl std::fmt::Display for LogParseError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			LogParseError::IoError(err) => write!(f, "{}", err),
			LogParseError::UnrecognizedFileExtension(ext) => {
				write!(f, "Unrecognized file extension: {}", ext.to_string_lossy())
			}
			LogParseError::NoFileExtension => write!(f, "No file extension"),
			LogParseError::UnrecognizedLogFile(path) => {
                write!(f, "File '{}' is not known log file. Parsing failed.", path.display())
            }
		}
	}
}

impl From<std::io::Error> for LogParseError {
	fn from(error: std::io::Error) -> Self {
		LogParseError::IoError(error)
	}
}

pub fn from_file(path: &std::path::PathBuf) -> Result<model::LogSource, LogParseError> {
	let extension = path.extension();
	if let Some(extension) = extension {
		match extension.to_string_lossy().to_lowercase().as_ref() {
			// ../logfiles/example.glog
			"glog" => {
				glog::from_file(&path).map_err(LogParseError::IoError)
			}
			// ../logfiles/logfile1.sfile
			"sfile" | "lfile" => sfile::from_file(&path).map_err(LogParseError::IoError),
			"txt" => {
                let file = std::fs::File::open(path)?;
                if robot_log::is_robot_log(file) {
                    robot_log::from_file(&path).map_err(LogParseError::IoError)
                } else {
                    Err(LogParseError::UnrecognizedLogFile(
                        path.clone(),
                    ))
                }
            }
			//TODO: Implement heuristic, more file types
			_ => Err(LogParseError::UnrecognizedFileExtension(
				extension.to_os_string(),
			)),
		}
	} else {
		//TODO: Implement heuristic
		Err(LogParseError::NoFileExtension)
	}
}
