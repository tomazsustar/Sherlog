use super::super::model;

use super::datetime_utils;

use std::collections::HashMap;
use std::io::BufReader;
use std::io::Read;
use std::mem;
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use std::sync::LazyLock;

// GLOG parser ----------------------------------------------------------------------

pub fn to_log_entries(reader: impl std::io::Read, root: model::LogSource) -> model::LogSource {
	let mut parser = GlogParser::new(root);

	let mut bufreader = BufReader::new(reader);
	let mut buffer = [0; 1];
	loop {
		if let Ok(bytes_read) = bufreader.read(&mut buffer) {
			if bytes_read == 0 {
				//log::info!("Len srcs {}, entrs {}", parser.log_sources.len(), parser.log_entries.len());
				break parser.finalize();
			} else {
				parser.read_byte(buffer[0]);
			}
		} else {
			break parser.finalize();
		}
	}
}

struct GlogParser {
	state: GlogParserState,
	buf: Vec<u8>,
	log_entry: model::LogEntry,
	sub_source: Option<i32>,
	log_entries: Vec<model::LogEntry>,
	log_sources: HashMap<String, model::LogSource>,
	invalid_bytes: usize,
	root: model::LogSource,
}

impl GlogParser {
	fn new(root: model::LogSource) -> GlogParser {
		GlogParser {
			state: GlogParserState::PreSection,
			buf: Vec::with_capacity(512),
			log_entry: model::LogEntry {
				..Default::default()
			},
			sub_source: None,
			log_entries: Vec::<model::LogEntry>::new(),
			log_sources: HashMap::<String, model::LogSource>::new(),
			invalid_bytes: 0,
			root,
		}
	}

	fn read_byte(&mut self, chr: u8) {
		self.state = match self.state {
			GlogParserState::PreSection => {
				if chr == b'[' {
					GlogParserState::SectionKind
				} else if chr == b'\r' || chr == b'\n' {
					GlogParserState::PreSection
				} else {
					self.invalid_bytes += 1;
					GlogParserState::PreSection
				}
			}
			GlogParserState::SectionKind => {
				if chr == b'|' {
					let kind_str = std::str::from_utf8(&self.buf);

					let kind = if let Ok(kind_str) = kind_str {
						match kind_str {
							"tq" => GlogSectionKind::TimestampMs, //controller only
							"s" => GlogSectionKind::Severity,
							"i" => GlogSectionKind::LogSource, //controller only
							"m" => GlogSectionKind::Message,
							"e" => GlogSectionKind::ErrorCode,      //sensor only
							"n" => GlogSectionKind::SessionId,      //sensor only
							"t" => GlogSectionKind::Timestamp100ns, //sensor only
							_ => {
								//TODO: Notify of invalid kind?
								log::warn!("UNRECOGNIZED kind: {}", &kind_str);
								GlogSectionKind::Unknown
							}
						}
					} else {
						//TODO: Notify of malformed UTF-8?
						log::warn!(
							"MALFORMED UTF-8 in kind string: {}",
							&String::from_utf8_lossy(&self.buf)
						);
						GlogSectionKind::Unknown
					};
					self.buf.clear();
					GlogParserState::SectionValue(kind)
				} else {
					self.buf.push(chr);
					GlogParserState::SectionKind
				}
			}
			GlogParserState::SectionValue(kind) => {
				self.buf.push(chr);
				if chr == b']' {
					GlogParserState::SectionValuePost1(kind)
				} else {
					GlogParserState::SectionValue(kind)
				}
			}
			GlogParserState::SectionValuePost1(kind) => {
				self.buf.push(chr);
				if chr == b':' {
					GlogParserState::SectionValuePost3(kind, 3, false)
				} else if chr == b'\r' {
					GlogParserState::SectionValuePost2(kind)
				} else if chr == b'\n' {
					GlogParserState::SectionValuePost3(kind, 3, true)
				} else if chr == b']' {
					GlogParserState::SectionValuePost1(kind)
				} else {
					GlogParserState::SectionValue(kind)
				}
			}
			GlogParserState::SectionValuePost2(kind) => {
				self.buf.push(chr);
				if chr == b'\n' {
					GlogParserState::SectionValuePost3(kind, 4, true)
				} else if chr == b']' {
					GlogParserState::SectionValuePost1(kind)
				} else {
					GlogParserState::SectionValue(kind)
				}
			}
			GlogParserState::SectionValuePost3(kind, suffix_cutoff, entry_done) => {
				self.buf.push(chr);
				if chr == b'[' {
					let value_str =
						String::from_utf8_lossy(&self.buf[0..self.buf.len() - suffix_cutoff]);

					match kind {
						GlogSectionKind::TimestampMs => {
							if let Ok(ts_milli) = value_str.parse::<u64>() {
								if let Some(datetime) = datetime_utils::from_timestamp_ms(ts_milli)
								{
									self.log_entry.timestamp = datetime;
								} else {
									//TODO: Notify of invalid datetime?
									log::warn!("MALFORMED Log ms timestamp: {}", ts_milli);
								}
							} else {
								//TODO: Notify of invalid timestamp?
								log::warn!("MALFORMED Log ms timestamp value: {}", value_str);
							}
						}
						GlogSectionKind::Severity => {
							if let Ok(glog_sev_u32) = value_str.parse::<u32>() {
								if let Some(glog_sev) = GlogSeverity::from_u32(glog_sev_u32) {
									self.log_entry.severity = normalize_glog_sev(glog_sev);
								} else {
									//TODO: Notify of invalid severity?
									log::warn!("INVALID Log severity: {}", value_str);
								}
							} else {
								//TODO: Notify of malformed severity?
								log::warn!("MALFORMED Log severity: {}", value_str);
							}
						}
						GlogSectionKind::LogSource => {
							if let Ok(parsed_sub_source) = value_str.parse::<i32>() {
								self.sub_source = Some(parsed_sub_source);
							//self.log_entry.custom_fields.insert(
							//	std::borrow::Cow::Borrowed("LogSource"),
							//	model::CustomField::Int32(parsed_sub_source),
							//);
							} else {
								//TODO: Notify of malformed sub-source?
								log::warn!("MALFORMED Log sub-source: {}", value_str);
							}
						}
						GlogSectionKind::Message => {
							if let std::borrow::Cow::Owned(owned_str) = &value_str {
								log::warn!("MALFORMED UTF-8 in Message: {}", owned_str);
							}
							self.log_entry.message = value_str.to_string();
						}
						GlogSectionKind::Timestamp100ns => {
							if let Ok(gcom_datetime) = value_str.parse::<u64>() {
								if let Some(datetime) = datetime_utils::from_100ns(gcom_datetime) {
									self.log_entry.timestamp = datetime;
								} else {
									//TODO: Notify of invalid datetime?
									log::warn!("MALFORMED Log 100ns datetime: {}", gcom_datetime);
								}
							} else {
								//TODO: Notify of invalid 100ns value?
								log::warn!("MALFORMED Log 100ns value: {}", value_str);
							}
						}
						GlogSectionKind::ErrorCode => {
							//TODO: Expose ErrorCode to user
						}
						GlogSectionKind::SessionId => {
							//TODO: Handle session ID, in particular sorting
							//with session ID instead of timestamp
							if let Ok(parsed_session_id) = value_str.parse::<u32>() {
								self.log_entry.custom_fields.insert(
									std::borrow::Cow::Borrowed("SessionId"),
									model::CustomField::UInt32(parsed_session_id),
								);
							} else {
								//TODO: Notify of malformed session id?
								log::warn!("MALFORMED Session ID: {}", value_str);
							}
						}
						GlogSectionKind::Unknown => (),
					}
					if entry_done {
						let log_entry = mem::replace(
							&mut self.log_entry,
							model::LogEntry {
								..Default::default()
							},
						);
						if let Some(sub_source) = self.sub_source {
							//Log entry specified a log sub-source

							//Note: We don't actually use QNX log source IDs properly
							//All we do is prepend source string name, plus colon and space

							let offset = log_entry.message.find(": ");
							let source_name = if let Some(offset) = offset {
								let name_slice = &log_entry.message[0..offset];
								if !name_slice.is_empty() {
									Some(std::borrow::Cow::from(name_slice))
								} else {
									None
								}
							} else {
								None
							}
							.unwrap_or_else(|| {
								std::borrow::Cow::from(format!("Unknown ({})", sub_source))
							});

							let source_option = self.log_sources.get_mut(source_name.as_ref());
							if let Some(source) = source_option {
								//Log sub-source exists, push log entry
								let children = &mut source.children;
								match children {
									model::LogSourceContents::Entries(v) => {
										v.push(log_entry);
									}
									_ => unreachable!(), //We only insert LogSourceContents::Entries
								}
							} else {
								//Log sub-source does not yet exist
								self.log_sources.insert(
									source_name.to_string(),
									model::LogSource {
										name: source_name.to_string(),
										children: {
											model::LogSourceContents::Entries(vec![log_entry])
										},
									},
								);
							}
						} else {
							//Log entry did not specify a log sub-source
							self.log_entries.push(log_entry);
						}
						self.sub_source = None;
					}
					self.buf.clear();
					GlogParserState::SectionKind
				} else if chr == b']' {
					GlogParserState::SectionValuePost1(kind)
				} else {
					GlogParserState::SectionValue(kind)
				}
			}
		};
	}

	fn finalize(mut self) -> model::LogSource {
		if self.invalid_bytes > 0 {
			//TODO: Invalid bytes?
			log::warn!("INVALID bytes encountered, count: {}", self.invalid_bytes);
		}
		match self.state {
			GlogParserState::PreSection => {
				//Log file empty
			}
			GlogParserState::SectionKind => {
				//TODO: Notify of cut off kind?
				log::warn!("CUT OFF last log message (kind)");
			}
			GlogParserState::SectionValue(_) => {
				//TODO: Notify of cut off kind?
				log::warn!("CUT OFF last log message (value)");
			}
			GlogParserState::SectionValuePost1(_) => {
				//Finish parsing section
				self.read_byte(b'\n');
				self.read_byte(b'[');
			}
			GlogParserState::SectionValuePost2(_) => {
				//Finish parsing section
				self.read_byte(b'\n');
				self.read_byte(b'[');
			}
			GlogParserState::SectionValuePost3(_, _, _) => {
				//Finish parsing section
				self.read_byte(b'[');
			}
		};

		if self.log_sources.is_empty() {
			//If no log message specified a source, we put the entries directly into the root
			self.root.children = model::LogSourceContents::Entries(self.log_entries);
		} else {
			let mut v = Vec::<model::LogSource>::with_capacity(
				self.log_sources.len() + !self.log_entries.is_empty() as usize,
			);
			for (_, sub_source) in self.log_sources {
				v.push(sub_source);
			}

			if !self.log_entries.is_empty() {
				let sub_source = model::LogSource {
					name: "Unknown (None)".to_string(),
					children: { model::LogSourceContents::Entries(self.log_entries) },
				};
				v.push(sub_source);
			}

			//Case insensitive sort by log source name
			v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
			self.root.children = model::LogSourceContents::Sources(v);
		}

		self.root
	}
}

#[derive(Copy, Clone)]
enum GlogParserState {
	PreSection,                                      //expect '[', ignore '\r' or '\n'
	SectionKind,                                     //expect kind until '|' (kind may not contain '|')
	SectionValue(GlogSectionKind),                   //expect value until ']'
	SectionValuePost1(GlogSectionKind),              //expect ':' or '\r' or '\n'
	SectionValuePost2(GlogSectionKind),              //expect '\n'
	SectionValuePost3(GlogSectionKind, usize, bool), //expect ']', process line
}

fn normalize_glog_sev(glog_sev: GlogSeverity) -> model::LogLevel {
	match glog_sev {
		GlogSeverity::Critical => model::LogLevel::Critical,
		GlogSeverity::Hardware => model::LogLevel::Critical,
		GlogSeverity::Error => model::LogLevel::Error,
		GlogSeverity::Warning => model::LogLevel::Warning,
		GlogSeverity::Info => model::LogLevel::Info,
		GlogSeverity::None => model::LogLevel::Debug,
		//Note: We map GlogSeverity::None to Debug.
		//GlogSeverity::None is often used to dump unstructured
		//log output whose severity is not known. Classifying that
		//as critical would be misleading, as it creates the
		//impression that there is a large number of critical errors.
		//A log message with this severity could be anything, from
		//critical error all the way to verbose debug data!!
		//
		//In my opinion, GlogSeverity::None shouldn't even exist,
		//as it allows unstructured log data inside a structured log
		//and delegates the difficult problem of log classification
		//to the outside world which knows even less about the true
		//severity of the messages. Therefore, firmware that logs with
		//GlogSeverity::None shall be considered buggy and should be fixed.
		//The usage of the GLOG severity SEV_NONE in the firmware's
		//C++ code shall be replaced with the appropriate severity!
		//Only sensor firmware contains this bug, controller firmware
		//does it right by virtue of using QNX's slogger/slogger2.
	}
}

enum GlogSeverity {
	Critical = 0,
	Hardware = 1,
	Error = 2,
	Warning = 3,
	Info = 4,
	None = 5,
}

impl GlogSeverity {
	fn from_u32(value: u32) -> Option<GlogSeverity> {
		match value {
			0 => Some(GlogSeverity::Critical),
			1 => Some(GlogSeverity::Hardware),
			2 => Some(GlogSeverity::Error),
			3 => Some(GlogSeverity::Warning),
			4 => Some(GlogSeverity::Info),
			5 => Some(GlogSeverity::None),
			_ => None,
		}
	}
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum GlogSectionKind {
	TimestampMs,
	Severity,
	LogSource,
	Message,
	Timestamp100ns,
	ErrorCode,
	SessionId,
	Unknown,
}

// Some log entries say:
// Setting EtherCAT time [delta = 1562060032100954112 ns].
// Others say (omitting the dot):
// Setting EtherCAT time [delta = 1562060032100954112 ns]
// Other time sources (e.g. gPTP) use the same format:
// Setting gPTP time [delta = 1672552120214243291 ns]
static TIME_DELTA_RE: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^Setting \S+ time \[delta = (-?\d+) ns\]\.?$").unwrap());

// Returns the delta string if `message` is a time delta log entry.
fn time_delta(message: &str) -> Option<&str> {
	// Cheap pre-check so the regex only runs on candidate lines
	if !message.starts_with("Setting ") {
		return None;
	}
	TIME_DELTA_RE
		.captures(message)
		.map(|captures| captures.get(1).unwrap().as_str())
}

pub fn adjust_sensor_timestamps(source: &mut model::LogSource) {
	match &mut source.children {
		model::LogSourceContents::Sources(v) => {
			for mut source in v {
				adjust_sensor_timestamps(&mut source);
			}
		}
		model::LogSourceContents::Entries(v) => {
			log::info!("Adjust sensor timestamps: {:?}", source.name);

			#[derive(Debug, PartialEq)]
			struct Correction {
				session_id: u32,
				delta: i64,
			}
			let mut active_correction: Option<Correction> = None;
			// Reverse iterate, from the newest to the oldest entry
			for entry in v.iter_mut().rev() {
				if let Some(field) = entry.custom_fields.get("SessionId") {
					if let model::CustomField::UInt32(session_id) = field {
						if let Some(delta) = time_delta(&entry.message) {
							//log::info!(
							//	"Found EtherCAT time correction entry: session: {}, msg: {}",
							//	session_id,
							//	entry.message
							//);
							if let Ok(delta) = delta.parse::<i64>() {
								let old_correction = mem::replace(
									&mut active_correction,
									Some(Correction {
										session_id: *session_id,
										delta,
									}),
								);

								let old_session_id_opt =
									if let Some(old_correction) = &old_correction {
										Some(old_correction.session_id)
									} else {
										None
									};

								if old_correction == active_correction {
									log::warn!(
										"Overwriting EtherCAT Time with same content! {:?}",
										active_correction
									);
								} else if old_session_id_opt == Some(*session_id) {
									log::warn!(
										"Overwriting EtherCAT Time! Old: {:?}, New: {:?}",
										old_correction,
										active_correction
									);
								} else {
									// This is the happy path for reading timestamp corrections.
									// Happens when:
									// - The very first correction is read
									// - A valid correction is read after the last one was invalidated by e.g. a session change
									// - A valid correction replaces a previous valid correction due to session change
									//log::info!(
									//	"Read fresh timestamp correction: {:?}, session ID: {}",
									//	active_correction,
									//	session_id
									//);
								}
							} else {
								log::warn!("could not parse time delta {}", delta);
								active_correction = None;
							}
						} else {
							if let Some(correction) = &active_correction {
								if *session_id == correction.session_id {
									// Timestamps before 01-01-2001 00:00:00.000000 are not realistic because the device did not exist back then.
									// We can safely assume that these are relative timestamps that are not yet corrected with EtherCAT time.
									// It is also reasonable to assume that a device receives its EtherCAT time within 2 years (or never).
									if entry.timestamp
										< DateTime::<Utc>::from_utc(
											NaiveDateTime::from_timestamp_opt(978_300_000, 0)
												.unwrap(),
											Utc,
										) {
										//Divide delta by 100 to convert from 1ns to 100ns ticks, which is the default GCOM timespan measurement
										if let Some(corrected_timestamp) =
											datetime_utils::add_offset_100ns(
												entry.timestamp,
												correction.delta / 100,
											) {
											entry.timestamp = corrected_timestamp;
										} else {
											log::warn!(
												"could not correct timestamp with offset: {}",
												correction.delta
											);
										}
									}
								} else {
									// We moved on to a different session. Scrap active timestamp correction.
									active_correction = None;
								}
							} else {
								// This either happens if we encounter an already corrected timestamp and haven't yet
								// encountered the log entry that specifies the time delta.
								// Or else, it happens if the bus never connected, so the device never got the EtherCAT offset.
								// This second case is also a normal thing to occur over the lifetime of a device,
								// but we have to think about how to sort these log lines as their timestamp remains around 1970.
								//log::warn!("Could not find EtherCAT offset for {}!", entry.message);
							}
						}
					} else {
						panic!("Wrong type for session ID!");
					}
				} else {
					// "sensorbase_BaseboardSpecialLogs_1_v.glog" lacks session ID, these logs are special
					log::warn!(
						"No session ID found for sensor log entry: {}",
						entry.message
					);
					active_correction = None;
				}
			}
		}
	}
}

// from_file -------------------------------------------------------------------------
// Reads a GLOG file from disk and returns the parsed log source
// with all its log entries and sub-sources.
// Adjusts sensor timestamps based on EtherCAT time corrections found in the log.
// Returns std::io::Error on file read errors.
// ------------------------------------------------------------------------------------
pub fn from_file(path: &std::path::PathBuf) -> Result<model::LogSource, std::io::Error> {
    let file = std::fs::File::open(&path)?;
    let root = model::LogSource {
        name: path.file_name().unwrap().to_string_lossy().to_string(),
        children: { model::LogSourceContents::Entries(Vec::<model::LogEntry>::new()) },
    };
    let mut log_source = to_log_entries(file, root);
    adjust_sensor_timestamps(&mut log_source);
    Ok(log_source)
}
// Tests for the GLOG parser and the sensor timestamp correction.
//
// GLOG files are written by firmware that can lose power at any time. The tests cover
// the malformed input the parser was hardened against over time: truncated entries,
// missing line endings, garbage and flash padding (NUL / 0xFF bytes), brackets inside
// messages, invalid field values and controller sub-sources.
//
// Tests marked #[ignore = "known bug: ..."] describe the correct behavior for cases the
// parser currently gets wrong. Run them with: cargo test -- --ignored
#[cfg(test)]
mod tests {
	use super::*;

	#[derive(Debug, PartialEq)]
	struct Entry {
		timestamp: DateTime<Utc>,
		severity: model::LogLevel,
		session_id: Option<u32>,
		message: String,
	}

	fn parse(input: &[u8]) -> model::LogSource {
		let root = model::LogSource {
			name: "root".to_string(),
			children: model::LogSourceContents::Entries(Vec::new()),
		};
		to_log_entries(input, root)
	}

	fn to_entries(v: &[model::LogEntry]) -> Vec<Entry> {
		v.iter()
			.map(|e| Entry {
				timestamp: e.timestamp,
				severity: match e.severity {
					model::LogLevel::Critical => model::LogLevel::Critical,
					model::LogLevel::Error => model::LogLevel::Error,
					model::LogLevel::Warning => model::LogLevel::Warning,
					model::LogLevel::Info => model::LogLevel::Info,
					model::LogLevel::Debug => model::LogLevel::Debug,
					model::LogLevel::Trace => model::LogLevel::Trace,
				},
				session_id: match e.custom_fields.get("SessionId") {
					Some(model::CustomField::UInt32(id)) => Some(*id),
					_ => None,
				},
				message: e.message.clone(),
			})
			.collect()
	}

	// Entries of a source without sub-sources
	fn entries(source: &model::LogSource) -> Vec<Entry> {
		match &source.children {
			model::LogSourceContents::Entries(v) => to_entries(v),
			model::LogSourceContents::Sources(_) => panic!("expected entries, got sub-sources"),
		}
	}

	// (name, entries) of each sub-source
	fn sub_sources(source: &model::LogSource) -> Vec<(String, Vec<Entry>)> {
		match &source.children {
			model::LogSourceContents::Sources(v) => v.iter().map(|s| (s.name.clone(), entries(s))).collect(),
			model::LogSourceContents::Entries(_) => panic!("expected sub-sources, got entries"),
		}
	}

	fn messages(source: &model::LogSource) -> Vec<String> {
		entries(source).into_iter().map(|e| e.message).collect()
	}

	fn ts100(gcom_datetime: u64) -> DateTime<Utc> {
		datetime_utils::from_100ns(gcom_datetime).unwrap()
	}

	fn ts_ms(timestamp_ms: u64) -> DateTime<Utc> {
		datetime_utils::from_timestamp_ms(timestamp_ms).unwrap()
	}

	fn epoch() -> DateTime<Utc> {
		model::LogEntry::default().timestamp
	}

	fn sensor(session_id: u32, ts: u64, severity: u32, message: &str) -> String {
		format!("[n|{}]:[t|{}]:[e|5420000]:[s|{}]:[m|{}]", session_id, ts, severity, message)
	}

	const T1: u64 = 639265188101410000; // 2026-10-02 06:20:10.141
	const T2: u64 = 639265188101420000; // 2026-10-02 06:20:10.142
	const T_BOOT: u64 = 621355968151283476; // 1970-01-01 00:00:15.128, before time sync

	fn first() -> String {
		sensor(5, T1, 3, "first")
	}

	fn second() -> String {
		sensor(5, T2, 4, "second")
	}

	// Well-formed input --------------------------------------------------------------

	#[test]
	fn sensor_entries_crlf() {
		let source = parse(format!("{}\r\n{}\r\n", first(), second()).as_bytes());
		assert_eq!(
			entries(&source),
			vec![
				Entry { timestamp: ts100(T1), severity: model::LogLevel::Warning, session_id: Some(5), message: "first".to_string() },
				Entry { timestamp: ts100(T2), severity: model::LogLevel::Info, session_id: Some(5), message: "second".to_string() },
			]
		);
	}

	#[test]
	fn sensor_entries_lf() {
		let crlf = parse(format!("{}\r\n{}\r\n", first(), second()).as_bytes());
		let lf = parse(format!("{}\n{}\n", first(), second()).as_bytes());
		assert_eq!(entries(&lf), entries(&crlf));
	}

	#[test]
	fn controller_entry_ms_timestamp() {
		let source = parse(b"[tq|000001790853428348]:[s|4]:[m|msg]\n");
		assert_eq!(
			entries(&source),
			vec![Entry { timestamp: ts_ms(1790853428348), severity: model::LogLevel::Info, session_id: None, message: "msg".to_string() }]
		);
	}

	#[test]
	fn severities() {
		let source = parse(b"[s|0]:[m|a]\r\n[s|1]:[m|b]\r\n[s|2]:[m|c]\r\n[s|3]:[m|d]\r\n[s|4]:[m|e]\r\n[s|5]:[m|f]\r\n");
		let severities: Vec<model::LogLevel> = entries(&source).into_iter().map(|e| e.severity).collect();
		assert_eq!(
			severities,
			vec![
				model::LogLevel::Critical, // Critical
				model::LogLevel::Critical, // Hardware
				model::LogLevel::Error,
				model::LogLevel::Warning,
				model::LogLevel::Info,
				model::LogLevel::Debug, // None: used by buggy firmware for unstructured output
			]
		);
	}

	#[test]
	fn entry_without_optional_fields() {
		let source = parse(b"[m|only message]\r\n");
		assert_eq!(
			entries(&source),
			vec![Entry { timestamp: epoch(), severity: model::LogLevel::Error, session_id: None, message: "only message".to_string() }]
		);
	}

	// Empty input and line endings ---------------------------------------------------

	#[test]
	fn empty_input() {
		assert!(entries(&parse(b"")).is_empty());
	}

	#[test]
	fn only_line_breaks() {
		assert!(entries(&parse(b"\r\n\n\r\n\n")).is_empty());
	}

	#[test]
	fn line_breaks_before_first_entry() {
		assert_eq!(messages(&parse(format!("\r\n\n{}\r\n", first()).as_bytes())), vec!["first"]);
	}

	#[test]
	fn last_entry_without_line_break() {
		assert_eq!(messages(&parse(format!("{}\r\n{}", first(), second()).as_bytes())), vec!["first", "second"]);
	}

	#[test]
	fn last_entry_with_cr_only() {
		assert_eq!(messages(&parse(format!("{}\r\n{}\r", first(), second()).as_bytes())), vec!["first", "second"]);
	}

	#[test]
	#[ignore = "known bug: an empty line between entries makes the parser lose all following entries"]
	fn empty_lines_between_entries() {
		let source = parse(format!("{}\r\n\r\n\r\n{}\r\n\r\n", first(), second()).as_bytes());
		assert_eq!(messages(&source), vec!["first", "second"]);
	}

	// Truncated input (power loss while writing) -------------------------------------

	#[test]
	fn cut_off_in_kind() {
		assert_eq!(messages(&parse(format!("{}\r\n[n|5]:[t|1]:[m", first()).as_bytes())), vec!["first"]);
	}

	#[test]
	fn cut_off_in_value() {
		assert_eq!(messages(&parse(format!("{}\r\n[n|5]:[t|1]:[m|half mess", first()).as_bytes())), vec!["first"]);
	}

	#[test]
	fn cut_off_after_open_bracket() {
		assert_eq!(messages(&parse(format!("{}\r\n[n|5]:[", first()).as_bytes())), vec!["first"]);
	}

	#[test]
	fn cut_off_after_section_separator() {
		assert_eq!(messages(&parse(format!("{}\r\n[n|5]:[t|{}]:", first(), T2).as_bytes())), vec!["first"]);
	}

	#[test]
	fn cut_off_after_closing_bracket() {
		// The last section is complete, only the line break is missing
		let source = parse(format!("{}\r\n[n|5]:[t|{}]:[s|4]:[m|last]", first(), T2).as_bytes());
		assert_eq!(messages(&source), vec!["first", "last"]);
	}

	#[test]
	fn cut_off_at_every_position_does_not_panic() {
		let input = format!("{}\r\n{}\r\n{}\n", first(), second(), "[tq|000001790853428348]:[s|4]:[i|13]:[m|Name: msg]");
		let input = input.as_bytes();
		for len in 0..=input.len() {
			parse(&input[..len]);
		}
		// Cutting off anywhere after the first entry's line break keeps the first entry intact
		let first_end = first().len() + 2;
		for len in first_end..=input.len() {
			let source = parse(&input[..len]);
			let first_entry = match &source.children {
				model::LogSourceContents::Entries(v) => to_entries(v).into_iter().next(),
				model::LogSourceContents::Sources(v) => v.iter().find(|s| s.name == "Unknown (None)").and_then(|s| entries(s).into_iter().next()),
			};
			assert_eq!(first_entry.map(|e| e.message), Some("first".to_string()), "cut off at {}", len);
		}
	}

	// Garbage and flash padding ------------------------------------------------------

	#[test]
	fn garbage_before_first_entry() {
		assert_eq!(messages(&parse(format!("xyz{}\r\n", first()).as_bytes())), vec!["first"]);
	}

	#[test]
	fn nul_padding_inside_cut_off_entry() {
		// Power loss in the middle of an entry, flash padding, then reboot
		let mut input = b"[n|5]:[t|639265188101410000]:[e|0]:[s|3]:[m|cut".to_vec();
		input.extend(std::iter::repeat(0u8).take(16));
		input.extend(format!("{}\r\n", sensor(6, T_BOOT, 4, "after reboot")).as_bytes());
		assert_eq!(messages(&parse(&input)), vec!["after reboot"]);
	}

	#[test]
	#[ignore = "known bug: the entry before NUL padding is lost and the next entry gets its session ID"]
	fn nul_padding_between_sessions() {
		// Real pattern after power loss: the flash contains NUL bytes between the last entry of the old
		// session and the first entry after reboot (e.g. telescope_LoggerAtr_1_p.glog: 209 NUL bytes)
		let mut input = format!("{}\r\n", first()).into_bytes();
		input.extend(std::iter::repeat(0u8).take(209));
		input.extend(format!("{}\r\n", sensor(6, T_BOOT, 4, "after reboot")).as_bytes());
		assert_eq!(
			entries(&parse(&input)),
			vec![
				Entry { timestamp: ts100(T1), severity: model::LogLevel::Warning, session_id: Some(5), message: "first".to_string() },
				Entry { timestamp: ts100(T_BOOT), severity: model::LogLevel::Info, session_id: Some(6), message: "after reboot".to_string() },
			]
		);
	}

	#[test]
	#[ignore = "known bug: the entry before 0xFF padding is lost and the next entry gets its session ID"]
	fn ff_padding_between_sessions() {
		let mut input = format!("{}\r\n", first()).into_bytes();
		input.extend(std::iter::repeat(0xFFu8).take(16));
		input.extend(format!("{}\r\n", sensor(6, T_BOOT, 4, "after reboot")).as_bytes());
		let source = parse(&input);
		assert_eq!(messages(&source), vec!["first", "after reboot"]);
		assert_eq!(entries(&source)[1].session_id, Some(6));
	}

	#[test]
	#[ignore = "known bug: NUL padding at the end of a file drops the last entry"]
	fn nul_padding_at_end() {
		let mut input = format!("{}\r\n", first()).into_bytes();
		input.extend(std::iter::repeat(0u8).take(16));
		assert_eq!(messages(&parse(&input)), vec!["first"]);
	}

	#[test]
	#[ignore = "known bug: a garbage line between entries drops the entry before it"]
	fn garbage_line_between_entries() {
		let source = parse(format!("{}\r\nxyz\r\n{}\r\n", first(), second()).as_bytes());
		assert_eq!(messages(&source), vec!["first", "second"]);
	}

	// Brackets and line breaks inside messages ---------------------------------------

	#[test]
	fn brackets_in_message() {
		assert_eq!(messages(&parse(format!("{}\r\n", sensor(5, T1, 4, "[M7 Log] text")).as_bytes())), vec!["[M7 Log] text"]);
	}

	#[test]
	fn message_ending_with_bracket() {
		// Real example: "kernel: Booting Linux on physical CPU 0x0000000000 [0x410fd034]]"
		assert_eq!(messages(&parse(format!("{}\r\n", sensor(5, T1, 4, "CPU [0x410fd034]")).as_bytes())), vec!["CPU [0x410fd034]"]);
	}

	#[test]
	fn closing_bracket_and_colon_in_message() {
		assert_eq!(messages(&parse(format!("{}\r\n", sensor(5, T1, 4, "a]:b")).as_bytes())), vec!["a]:b"]);
	}

	#[test]
	fn line_breaks_in_message() {
		// Real example: "Init command V2 completed successfully. (0 Errors, 0 Warnings, 0 Infos)\n]"
		assert_eq!(messages(&parse(format!("{}\r\n", sensor(5, T1, 4, "line1\nline2\n")).as_bytes())), vec!["line1\nline2\n"]);
	}

	#[test]
	fn section_separator_in_message_ends_the_message() {
		// "]\n[" inside a message is indistinguishable from the end of the entry. The rest is not a valid section.
		assert_eq!(messages(&parse(format!("{}\r\n", sensor(5, T1, 4, "a]\n[b")).as_bytes())), vec!["a"]);
	}

	// Invalid fields -----------------------------------------------------------------

	#[test]
	fn unknown_kind_is_ignored() {
		let source = parse(format!("[x|whatever]:{}\r\n", sensor(5, T1, 4, "msg")).as_bytes());
		assert_eq!(
			entries(&source),
			vec![Entry { timestamp: ts100(T1), severity: model::LogLevel::Info, session_id: Some(5), message: "msg".to_string() }]
		);
	}

	#[test]
	fn invalid_utf8_in_message() {
		let source = parse(b"[n|5]:[t|639265188101410000]:[s|4]:[m|bad \xC3\x28 utf8]\r\n");
		assert_eq!(messages(&source), vec!["bad \u{FFFD}( utf8"]);
	}

	#[test]
	fn invalid_utf8_in_kind() {
		let source = parse(b"[\xC3\x28|1]:[t|639265188101410000]:[s|4]:[m|msg]\r\n");
		assert_eq!(
			entries(&source),
			vec![Entry { timestamp: ts100(T1), severity: model::LogLevel::Info, session_id: None, message: "msg".to_string() }]
		);
	}

	#[test]
	fn invalid_timestamp() {
		let source = parse(b"[n|5]:[t|abc]:[s|4]:[m|msg]\r\n");
		assert_eq!(entries(&source)[0].timestamp, epoch());
	}

	#[test]
	fn invalid_ms_timestamp() {
		let source = parse(b"[tq|abc]:[s|4]:[m|msg]\n");
		assert_eq!(entries(&source)[0].timestamp, epoch());
	}

	#[test]
	fn invalid_severity() {
		for input in [&b"[n|5]:[t|639265188101410000]:[s|9]:[m|msg]\r\n"[..], &b"[n|5]:[t|639265188101410000]:[s|x]:[m|msg]\r\n"[..]] {
			let source = parse(input);
			assert_eq!(entries(&source)[0].severity, model::LogLevel::Error);
			assert_eq!(entries(&source)[0].message, "msg");
		}
	}

	#[test]
	fn invalid_session_id() {
		let source = parse(b"[n|x]:[t|639265188101410000]:[s|4]:[m|msg]\r\n");
		assert_eq!(entries(&source)[0].session_id, None);
	}

	#[test]
	fn section_without_kind_separator() {
		assert!(entries(&parse(b"[m only message]\r\n")).is_empty());
	}

	// Controller sub-sources ---------------------------------------------------------

	#[test]
	fn sub_sources_by_message_prefix() {
		let source = parse(
			b"[tq|000001790853428348]:[s|4]:[i|13]:[m|zeta: a]\n\
			  [tq|000001790853428349]:[s|4]:[i|13]:[m|Alpha: b]\n\
			  [tq|000001790853428350]:[s|4]:[i|7]:[m|zeta: c]\n",
		);
		let sub_sources: Vec<(String, Vec<String>)> =
			sub_sources(&source).into_iter().map(|(name, v)| (name, v.into_iter().map(|e| e.message).collect())).collect();
		// Sorted by name, case insensitive. The source ID is not used, only the message prefix.
		assert_eq!(
			sub_sources,
			vec![
				("Alpha".to_string(), vec!["Alpha: b".to_string()]),
				("zeta".to_string(), vec!["zeta: a".to_string(), "zeta: c".to_string()]),
			]
		);
	}

	#[test]
	fn sub_source_without_name() {
		let source = parse(b"[tq|000001790853428348]:[s|4]:[i|13]:[m|no colon here]\n[tq|000001790853428349]:[s|4]:[i|13]:[m|: empty name]\n");
		let sub_sources = sub_sources(&source);
		assert_eq!(sub_sources.len(), 1);
		assert_eq!(sub_sources[0].0, "Unknown (13)");
		assert_eq!(sub_sources[0].1.len(), 2);
	}

	#[test]
	fn sub_source_with_invalid_id() {
		// An invalid source ID is ignored, the entry goes into the root
		let source = parse(b"[tq|000001790853428348]:[s|4]:[i|x]:[m|Name: msg]\n");
		assert_eq!(messages(&source), vec!["Name: msg"]);
	}

	#[test]
	fn entries_with_and_without_sub_source() {
		let source = parse(b"[tq|000001790853428348]:[s|4]:[i|13]:[m|Name: a]\n[tq|000001790853428349]:[s|4]:[m|no source]\n");
		let names: Vec<String> = sub_sources(&source).into_iter().map(|(name, _)| name).collect();
		assert_eq!(names, vec!["Name", "Unknown (None)"]);
	}

	#[test]
	fn sub_source_id_does_not_leak_to_next_entry() {
		let source = parse(b"[tq|000001790853428348]:[s|4]:[i|13]:[m|Name: a]\n[tq|000001790853428349]:[s|4]:[m|Other: b]\n");
		let sub_sources = sub_sources(&source);
		assert_eq!(sub_sources[1].0, "Unknown (None)");
		assert_eq!(sub_sources[1].1[0].message, "Other: b");
	}

	// Sensor timestamp correction ----------------------------------------------------

	const DELTA: i64 = 1790921687981282700; // ns

	fn corrected(raw: u64) -> DateTime<Utc> {
		datetime_utils::add_offset_100ns(ts100(raw), DELTA / 100).unwrap()
	}

	fn adjusted(lines: &[String]) -> Vec<Entry> {
		let mut input = String::new();
		for line in lines {
			input.push_str(line);
			input.push_str("\r\n");
		}
		let mut source = parse(input.as_bytes());
		adjust_sensor_timestamps(&mut source);
		entries(&source)
	}

	#[test]
	fn time_correction_ethercat() {
		let result = adjusted(&[
			sensor(5, T_BOOT, 4, "before sync"),
			sensor(5, T_BOOT + 1, 4, &format!("Setting EtherCAT time [delta = {} ns]", DELTA)),
			sensor(5, T1, 4, "after sync"),
		]);
		assert_eq!(result[0].timestamp, corrected(T_BOOT));
		assert_eq!(result[1].timestamp, ts100(T_BOOT + 1)); // The correction entry itself is not corrected
		assert_eq!(result[2].timestamp, ts100(T1));
	}

	#[test]
	fn time_correction_with_dot_and_other_time_sources() {
		for line in [
			format!("Setting EtherCAT time [delta = {} ns].", DELTA),
			format!("Setting gPTP time [delta = {} ns]", DELTA),
			format!("Setting 802.1AS time [delta = {} ns]", DELTA),
		] {
			let result = adjusted(&[sensor(5, T_BOOT, 4, "before sync"), sensor(5, T_BOOT + 1, 4, &line)]);
			assert_eq!(result[0].timestamp, corrected(T_BOOT), "{}", line);
		}
	}

	#[test]
	fn time_correction_only_within_session() {
		let result = adjusted(&[
			sensor(4, T_BOOT, 4, "other session"),
			sensor(5, T_BOOT, 4, "same session"),
			sensor(5, T_BOOT + 1, 4, &format!("Setting EtherCAT time [delta = {} ns]", DELTA)),
		]);
		assert_eq!(result[0].timestamp, ts100(T_BOOT));
		assert_eq!(result[1].timestamp, corrected(T_BOOT));
	}

	#[test]
	fn time_correction_only_before_2001() {
		let already_correct: u64 = 631139040000000000; // 2001-01-01 00:00:00
		let result = adjusted(&[
			sensor(5, already_correct, 4, "already correct"),
			sensor(5, T_BOOT + 1, 4, &format!("Setting EtherCAT time [delta = {} ns]", DELTA)),
		]);
		assert_eq!(result[0].timestamp, ts100(already_correct));
	}

	#[test]
	fn time_correction_uses_the_preceding_delta_of_a_session() {
		// Two corrections in the same session: entries are corrected with the next newer one
		let result = adjusted(&[
			sensor(5, T_BOOT, 4, "corrected with first delta"),
			sensor(5, T_BOOT + 1, 4, &format!("Setting EtherCAT time [delta = {} ns]", DELTA)),
			sensor(5, T_BOOT + 2, 4, "corrected with second delta"),
			sensor(5, T_BOOT + 3, 4, &format!("Setting EtherCAT time [delta = {} ns]", DELTA + 1_000_000_000)),
		]);
		assert_eq!(result[0].timestamp, corrected(T_BOOT));
		assert_eq!(
			result[2].timestamp,
			datetime_utils::add_offset_100ns(ts100(T_BOOT + 2), (DELTA + 1_000_000_000) / 100).unwrap()
		);
	}

	#[test]
	fn time_correction_without_delta() {
		let result = adjusted(&[sensor(5, T_BOOT, 4, "never synchronized")]);
		assert_eq!(result[0].timestamp, ts100(T_BOOT));
	}

	#[test]
	fn time_correction_with_invalid_delta() {
		let result = adjusted(&[
			sensor(5, T_BOOT, 4, "before sync"),
			sensor(5, T_BOOT + 1, 4, "Setting EtherCAT time [delta = 99999999999999999999 ns]"),
		]);
		assert_eq!(result[0].timestamp, ts100(T_BOOT));
	}

	#[test]
	fn time_correction_stops_at_entries_without_session() {
		let result = adjusted(&[
			sensor(5, T_BOOT, 4, "before gap"),
			format!("[t|{}]:[s|4]:[m|no session]", T_BOOT),
			sensor(5, T_BOOT + 1, 4, &format!("Setting EtherCAT time [delta = {} ns]", DELTA)),
		]);
		assert_eq!(result[0].timestamp, ts100(T_BOOT));
		assert_eq!(result[1].timestamp, ts100(T_BOOT));
	}
}
