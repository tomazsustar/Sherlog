extern crate chrono;
extern crate zip;

use chrono::prelude::DateTime;
use chrono::prelude::NaiveDateTime;
use chrono::prelude::Utc;
use std::path::PathBuf;

use super::super::model;
use super::glog;
use super::rds_log;
use super::robot_log;
use super::scanlib_log;
use super::xlog;

use std::collections::HashMap;

static SFILE_PASSWORD: Option<&'static str> = option_env!("SFILE_PASSWORD");

pub fn from_file(path: &std::path::PathBuf) -> Result<model::LogSource, std::io::Error> {
	let file = std::fs::File::open(&path)?;
	let mut archive = zip::ZipArchive::new(file)?;

	// Classify the archive entries by their names only. Only known log files become parse jobs,
	// all other files are never decompressed.
	let mut jobs = Vec::new();
	let mut glog_files = Vec::new();

	for i in 0..archive.len() {
		let file = archive.by_index_raw(i)?;
		let compressed_size = file.compressed_size();
		let outpath = PathBuf::from(file.name());
		let stem = outpath.file_stem().unwrap();
		let stem = stem.to_string_lossy();
		//log::info!("File contained: {}", &stem);

		// .ZIP specification, Version: 6.3.9, Paragraph 4.4.17 file name: (Variable)
		// All slashes MUST be forward slashes '/' as opposed to backwards slashes '\' [...]
		//
		// Therefore, we can safely match folders with `/`
		if outpath.starts_with("RDS/") {
			if let Some(extension) = outpath.extension() {
				match extension.to_string_lossy().as_ref() {
					"log" => {
						let kind = if stem.starts_with("ScanLib_") {
							log::info!("Log file (ScanLib): {}", &stem);
							ParseJobKind::ScanLib
						} else {
							log::info!("Log file (RDS): {}", &stem);
							ParseJobKind::Rds
						};
						jobs.push(ParseJob::single(kind, stem.to_string(), i, compressed_size));
					}
					unknown_extension => {
						log::warn!("Unknown extension in RDS folder: {}", unknown_extension)
					}
				}
			}
		} else if let Some(extension) = outpath.extension() {
			match extension.to_string_lossy().as_ref() {
				"glog" => {
					glog_files.push(ZipEntry {
						name: stem.to_string(),
						group_name: get_group_name(&stem),
						index: i,
						compressed_size,
					});
				}
				"xlog" => {
					//log::info!("XLOG: {}", &stem);
					jobs.push(ParseJob::single(ParseJobKind::Xlog, stem.to_string(), i, compressed_size));
				}
				"log" if file.name().contains("robot") => {
					log::info!("Robot Framework log: {}", file.name());
					jobs.push(ParseJob::single(ParseJobKind::Robot, stem.to_string(), i, compressed_size));
				}
				_ => (),
			}
		}
	}

	//Sort glog files by group (a group is the file name with its ring buffer ID / overview suffix removed)
	//Inside a group, reverse sort by file name (and thus buffer ID) to get chronological ordering of files.
	//The reason for this is that higher ring buffer ID means the file is older. Older log entries come first.
	//Note: Overview logs are interspersed with normal logs, therefore no chronological order between
	//the log entries of normal logs and overview logs can be established without inspecting timestamps.
	//Note: Sensor has logSync and logAsync, leading to minor violations of chronological order inside a file
	//and even e.g. between the end of one file and the start of another.
	//Due to these facts, this ordering is only mostly chronological with regards to individual log entries.
	//This chronological ordering is required to perform timestamp corrections for devices that do not have
	//a real-time clock and thus start with 1970 timestamps on boot-up before distributed clock time is set.
	//It is also required to preserve the order of log entries that were logged with the exact same timestamp.
	//
	//Example ordering:
	//Group name           File name
	//--------------------------------------
	//adm_LoggerAdm_p      adm_LoggerAdm_2_p
	//adm_LoggerAdm_p      adm_LoggerAdm_1_p
	//adm_LoggerAdm_v      adm_LoggerAdm_1_v
	//contr_FtcManager     contr_FtcManager_ov
	//contr_FtcManager     contr_FtcManager
	//contr_Hwa            contr_Hwa_4
	//contr_Hwa            contr_Hwa_3
	//contr_Hwa            contr_Hwa_2
	//contr_Hwa            contr_Hwa_1
	//contr_Hwa            contr_Hwa
	//
	//Note the sorting subtleties, especially with _p and _v logs, where the ID is not a suffix.

	glog_files.sort_unstable_by(|a, b| {
		a.group_name
			.cmp(&b.group_name)
			.then(a.name.cmp(&b.name).reverse())
	});

	// Each glog group becomes one job that reads the group's files as one concatenated stream
	for file in glog_files {
		let is_new_group = match jobs.last() {
			Some(job) => job.kind != ParseJobKind::Glog || job.name != file.group_name,
			None => true,
		};
		if is_new_group {
			log::info!("--------------------");
			log::info!("Glog file: {:?}", file);
			jobs.push(ParseJob {
				kind: ParseJobKind::Glog,
				name: file.group_name,
				indices: std::collections::VecDeque::new(),
				compressed_size: 0,
			});
		} else {
			log::info!("Glog file: {:?}", file);
		}
		let job = jobs.last_mut().unwrap();
		job.indices.push_back(file.index);
		job.compressed_size += file.compressed_size;
	}

	// Decompress and parse all jobs in parallel. The results come back in job order.
	let mut client_child_sources = Vec::new();
	let mut rds_child_sources = Vec::new();
	let mut scanlib_child_sources = Vec::new();
	let mut child_sources = Vec::new();
	for (kind, source) in parse_jobs_parallel(path, &jobs)? {
		match kind {
			ParseJobKind::Xlog | ParseJobKind::Robot => client_child_sources.push(source),
			ParseJobKind::Rds => rds_child_sources.push(source),
			ParseJobKind::ScanLib => scanlib_child_sources.push(source),
			ParseJobKind::Glog => child_sources.push(source),
		}
	}

	//Arrange Client logs into their respective channels
	let mut client_log_sources = HashMap::<String, model::LogSource>::new();
	for file_source in client_child_sources {
		let mut rsplitn_name = file_source.name.rsplitn(4, '_');

		let _date_time = if let Some(date_time) = rsplitn_name.next() {
			// Parse date_time. Example: "2021-03-09-08-07-25-8527"
			let padded_date_time = format!("{}00000", date_time); // Pad fractional seconds to nanoseconds
			if let Ok(ndt) =
				NaiveDateTime::parse_from_str(&padded_date_time, "%Y-%m-%d-%H-%M-%S-%f")
			{
				DateTime::<Utc>::from_utc(ndt, Utc)
			} else {
				// Invalid date_time format, parse error
				DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp_opt(0, 0).unwrap(), Utc)
			}
		} else {
			// rsplitn always returns at least one iterator element
			unreachable!()
		};

		let channel_name = if let Some(channel_name) = rsplitn_name.next() {
			channel_name
		} else {
			// Too few iterator elements (underscores)
			// Use entire file name (without ".xlog" extension) as fallback
			&file_source.name
		};

		let _process_id = if let Some(process_id) = rsplitn_name
			.next()
			.map_or(None, |pid_str| pid_str.parse::<u32>().ok())
		{
			process_id // Windows PIDs are stored in nonzero DWORD (u32)
		} else {
			0 // Invalid PID
		};

		let _application_name = if let Some(application_name) = rsplitn_name.next() {
			application_name
		} else {
			"Unknown application name"
		};

		//log::info!("{:?} {:?} {:?} {:?}", _date_time, channel_name, _process_id, _application_name);

		let source_option = client_log_sources.get_mut(channel_name);
		if let Some(source) = source_option {
			//Log sub-source exists, push contents
			let children = &mut source.children;
			match children {
				model::LogSourceContents::Entries(v) => {
					if let model::LogSourceContents::Entries(mut entries) = file_source.children {
						v.append(&mut entries);
					} else {
						unreachable!(); //If this panics, there is a bug in the XLOG parser
					}
				}
				_ => unreachable!(), //We only insert LogSourceContents::Entries
			}
		} else {
			//Log sub-source does not yet exist
			client_log_sources.insert(
				channel_name.to_string(),
				model::LogSource {
					name: channel_name.to_string(),
					children: {
						if let model::LogSourceContents::Entries(entries) = file_source.children {
							model::LogSourceContents::Entries(entries)
						} else {
							unreachable!(); //If this panics, there is a bug in the XLOG parser
						}
					},
				},
			);
		}
	}
	let mut client_child_sources = Vec::new();
	for (_, sub_source) in client_log_sources {
		client_child_sources.push(sub_source);
	}

	let mut contr_child_sources = Vec::new();
	let mut sensor_child_sources: std::vec::Vec<model::LogSource> = Vec::new();
	let mut cbox_child_sources = Vec::new();
	let mut probe_child_sources = Vec::new();
	let mut unknown_child_sources = Vec::new();

	for mut source in child_sources {
		//Controller logs
		if source.name.starts_with("contr_") {
			//Remove "contr_" to make it look nicer
			source.name = source.name.split_off(6);
			contr_child_sources.push(source);
			continue;
		}

		//Sensor logs
		let mut iter = source.name.splitn(2, '_');
		let board_name = iter.next().unwrap(); //First element always exists
		if let Some(log_name) = iter.next() {
			if board_name == "axis" ||
				board_name == "sensorbase" ||
				board_name == "telescope" ||
				board_name == "trigger" ||
				board_name == "adm" || // G
				board_name == "laseroven" || // G
				board_name == "wfd"	|| // W
				board_name == "dynamicadm" || // P
				board_name == "icbpower" || // P
				board_name.starts_with("cfm") || // L
				board_name == "laserctl" || // L
				board_name == "wlanmodule" || // L
				false
			{
				if sensor_child_sources.is_empty()
					|| sensor_child_sources.last().unwrap().name != board_name
				{
					let board_name_string = board_name.to_string();
					source.name = log_name.to_string();
					sensor_child_sources.push(model::LogSource {
						name: board_name_string,
						children: { model::LogSourceContents::Sources(vec![source]) },
					});
				} else {
					source.name = log_name.to_string();
					if let model::LogSourceContents::Sources(sources) =
						&mut sensor_child_sources.last_mut().unwrap().children
					{
						sources.push(source)
					} else {
						//We only pushed model::LogSourceContents::Sources
						unreachable!();
					}
				}
				continue;
			}
		}

		//Connect Box logs
		if source.name.starts_with("connectbox_") {
			//Remove "connectbox_" to make it look nicer
			source.name = source.name.split_off(11);
			cbox_child_sources.push(source);
			continue;
		}

		//Probe logs
		if source.name.starts_with("ap21_") {
			//Remove "ap21_" to make it look nicer
			source.name = source.name.split_off(5);
			probe_child_sources.push(source);
			continue;
		}

		//Unknown logs
		unknown_child_sources.push(source);
	}

	//Case insensitive sort by log source name
	contr_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
	sensor_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
	client_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
	cbox_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
	probe_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
	rds_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
	scanlib_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
	unknown_child_sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

	for mut source in &mut sensor_child_sources {
		glog::adjust_sensor_timestamps(&mut source);
	}
	for mut source in &mut cbox_child_sources {
		glog::adjust_sensor_timestamps(&mut source);
	}
	for mut source in &mut probe_child_sources {
		glog::adjust_sensor_timestamps(&mut source);
	}

	let contr_logs = model::LogSource {
		name: "Controller".to_string(),
		children: { model::LogSourceContents::Sources(contr_child_sources) },
	};
	let sensor_logs = model::LogSource {
		name: "Sensor".to_string(),
		children: { model::LogSourceContents::Sources(sensor_child_sources) },
	};
	let client_logs = model::LogSource {
		name: "Client".to_string(),
		children: { model::LogSourceContents::Sources(client_child_sources) },
	};

	let mut sources_vec = vec![client_logs, contr_logs, sensor_logs];

	if !cbox_child_sources.is_empty() {
		let cbox_logs = model::LogSource {
			name: "Connect Box".to_string(),
			children: { model::LogSourceContents::Sources(cbox_child_sources) },
		};
		sources_vec.push(cbox_logs);
	}

	if !probe_child_sources.is_empty() {
		let probe_logs = model::LogSource {
			name: "Probe".to_string(),
			children: { model::LogSourceContents::Sources(probe_child_sources) },
		};
		sources_vec.push(probe_logs);
	}

	if !rds_child_sources.is_empty() {
		let rds_logs = model::LogSource {
			name: "RDS".to_string(),
			children: { model::LogSourceContents::Sources(rds_child_sources) },
		};
		sources_vec.push(rds_logs);
	}

	if !scanlib_child_sources.is_empty() {
		let scanlib_logs = model::LogSource {
			name: "ScanLib".to_string(),
			children: { model::LogSourceContents::Sources(scanlib_child_sources) },
		};
		sources_vec.push(scanlib_logs);
	}

	if !unknown_child_sources.is_empty() {
		let unknown_logs = model::LogSource {
			name: "Unknown".to_string(),
			children: { model::LogSourceContents::Sources(unknown_child_sources) },
		};
		sources_vec.push(unknown_logs);
	}

	Ok(model::LogSource {
		name: path.file_name().unwrap().to_string_lossy().to_string(),
		children: { model::LogSourceContents::Sources(sources_vec) },
	})
}

struct ConcatZipReader<'a, R: std::io::Read + std::io::Seek> {
	archive: &'a mut zip::ZipArchive<R>,
	file: Option<zip::read::ZipFile<'a>>,
	indices: std::collections::VecDeque<usize>,
}

impl<'a, R: std::io::Read + std::io::Seek> ConcatZipReader<'a, R> {
	fn new(
		archive: &'a mut zip::ZipArchive<R>,
		indices: std::collections::VecDeque<usize>,
	) -> ConcatZipReader<'a, R> {
		ConcatZipReader {
			archive,
			file: None,
			indices,
		}
	}
}

impl<'a, R: std::io::Read + std::io::Seek> std::io::Read for ConcatZipReader<'a, R> {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		loop {
			if let Some(f) = &mut self.file {
				let result = f.read(buf);
				if let Ok(bytes) = result {
					if bytes == 0 {
						//file exhausted, retry with next file
						self.file = None;
						continue;
					}
				}
				//ordinary read from the file
				return result;
			} else {
				match self.indices.pop_front() {
					Some(idx) => {
						//Need to open new file
						let f = open_entry(self.archive, idx)?;
						unsafe {
							//Due to the fact that file references archive and both are in the same struct,
						 //this cannot be done in safe Rust
							self.file = Some(std::mem::transmute::<
								zip::read::ZipFile<'_>,
								zip::read::ZipFile<'a>,
							>(f));
						}
						//retry with newly opened file
						continue;
					}
					None => {
						//file list exhausted, end of reader
						return Ok(0);
					}
				}
			}
		}
	}
}

#[derive(Debug)]
struct ZipEntry {
	name: String,
	group_name: String,
	index: usize,
	compressed_size: u64,
}

fn open_entry<R: std::io::Read + std::io::Seek>(
	archive: &mut zip::ZipArchive<R>,
	index: usize,
) -> Result<zip::read::ZipFile<'_>, std::io::Error> {
	if let Some(password) = SFILE_PASSWORD {
		Ok(archive.by_index_decrypt(index, password.as_bytes())?.unwrap()) //TODO: 21.06.2020: Handle InvalidPassword!
	} else {
		Ok(archive.by_index(index)?)
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ParseJobKind {
	Xlog,
	Robot,
	Rds,
	ScanLib,
	Glog,
}

// A log source to be parsed from one or more archive entries
#[derive(Debug)]
struct ParseJob {
	kind: ParseJobKind,
	name: String,
	indices: std::collections::VecDeque<usize>, //Archive entries, read as one concatenated stream
	compressed_size: u64,                       //Used to schedule big jobs first
}

impl ParseJob {
	fn single(kind: ParseJobKind, name: String, index: usize, compressed_size: u64) -> ParseJob {
		ParseJob {
			kind,
			name,
			indices: std::collections::VecDeque::from(vec![index]),
			compressed_size,
		}
	}
}

fn parse_job<R: std::io::Read + std::io::Seek>(
	archive: &mut zip::ZipArchive<R>,
	job: &ParseJob,
) -> Result<Option<model::LogSource>, std::io::Error> {
	let root = model::LogSource {
		name: job.name.clone(),
		children: { model::LogSourceContents::Entries(Vec::<model::LogEntry>::new()) },
	};
	if job.kind == ParseJobKind::Glog {
		let reader = ConcatZipReader::new(archive, job.indices.clone());
		return Ok(Some(glog::to_log_entries(reader, root)));
	}
	let file = open_entry(archive, job.indices[0])?;
	let source = match job.kind {
		ParseJobKind::Xlog => xlog::to_log_entries(file, root),
		ParseJobKind::Rds => rds_log::to_log_entries(file, root),
		ParseJobKind::ScanLib => scanlib_log::to_log_entries(file, root),
		ParseJobKind::Robot => match robot_log::to_log_entries(file, "RobotFramework".to_string()) {
			Ok(source) => source,
			Err(e) => {
				log::warn!("Failed to parse Robot Framework log {}: {}", job.name, e);
				return Ok(None);
			}
		},
		ParseJobKind::Glog => unreachable!(),
	};
	Ok(Some(source))
}

// Parsing is limited by memory allocation and bandwidth, so more threads than this make it slower
const MAX_PARSE_THREADS: usize = 8;

// Parses all jobs on up to MAX_PARSE_THREADS CPU cores. Each thread opens its own handle to the archive.
// Returns the parsed sources in job order; jobs that produced no source are left out.
fn parse_jobs_parallel(
	path: &std::path::Path,
	jobs: &[ParseJob],
) -> Result<Vec<(ParseJobKind, model::LogSource)>, std::io::Error> {
	use std::sync::atomic::{AtomicUsize, Ordering};

	// Biggest jobs first, so that no big job is started last and delays the end
	let mut schedule: Vec<usize> = (0..jobs.len()).collect();
	schedule.sort_by_key(|&i| std::cmp::Reverse(jobs[i].compressed_size));

	let thread_count = std::thread::available_parallelism()
		.map_or(1, |n| n.get())
		.min(MAX_PARSE_THREADS)
		.min(jobs.len())
		.max(1);
	let next = AtomicUsize::new(0);

	let thread_results: Vec<Result<Vec<(usize, Option<model::LogSource>)>, std::io::Error>> =
		std::thread::scope(|scope| {
			let handles: Vec<_> = (0..thread_count)
				.map(|_| {
					scope.spawn(|| -> Result<Vec<(usize, Option<model::LogSource>)>, std::io::Error> {
						let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
						let mut parsed = Vec::new();
						loop {
							let n = next.fetch_add(1, Ordering::Relaxed);
							if n >= schedule.len() {
								return Ok(parsed);
							}
							let job_index = schedule[n];
							match parse_job(&mut archive, &jobs[job_index]) {
								Ok(source) => parsed.push((job_index, source)),
								Err(e) => {
									next.store(schedule.len(), Ordering::Relaxed); //Stop the other threads
									return Err(e);
								}
							}
						}
					})
				})
				.collect();
			handles
				.into_iter()
				.map(|handle| handle.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
				.collect()
		});

	let mut sources: Vec<Option<model::LogSource>> = (0..jobs.len()).map(|_| None).collect();
	for thread_result in thread_results {
		for (job_index, source) in thread_result? {
			sources[job_index] = source;
		}
	}
	Ok(jobs
		.iter()
		.zip(sources)
		.filter_map(|(job, source)| source.map(|source| (job.kind, source)))
		.collect())
}

fn get_group_name(s: &str) -> String {
	let mut s = s.to_string();
	let storage_type = if s.ends_with("_v") {
		//Virtual (in RAM)
		"_v"
	} else if s.ends_with("_p") {
		//Persistent (on Flash storage)
		"_p"
	} else {
		//Unknown storage type
		""
	};

	s.truncate(s.len() - storage_type.len());

	if let Some(offset) = s.rfind('_') {
		if s[offset + 1..s.len()].parse::<u32>().is_ok() {
			//Numbered logfile. Discard ring buffer index.
			s.truncate(offset);
		}
	}

	if s.ends_with("_ov") {
		//Overview logs are merged with normal logs.
		s.truncate(s.len() - "_ov".len());
	}

	//Restore storage type suffix
	s.push_str(storage_type);
	s
}
#[cfg(test)]
mod tests {
	use super::*;

	fn count_entries(source: &model::LogSource) -> usize {
		match &source.children {
			model::LogSourceContents::Sources(v) => v.iter().map(count_entries).sum(),
			model::LogSourceContents::Entries(v) => v.len(),
		}
	}

	// Opens every sfile/lfile in the local test/ directory and measures the time.
	// The sample files are not committed (see .gitignore), so the test passes without checking anything when there are none.
	// Encrypted files require SFILE_PASSWORD to be set at build time.
	// Run with: cargo test --release open_sfile -- --nocapture
	#[test]
	fn open_sfile() {
		let test_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test");
		let mut paths: Vec<PathBuf> = std::fs::read_dir(&test_dir)
			.map(|dir| {
				dir.filter_map(|entry| entry.ok().map(|entry| entry.path()))
					.filter(|path| {
						path.extension().map_or(false, |ext| {
							ext.eq_ignore_ascii_case("sfile") || ext.eq_ignore_ascii_case("lfile")
						})
					})
					.collect()
			})
			.unwrap_or_default();
		paths.sort();

		if paths.is_empty() {
			println!("No sample files in {}, skipping", test_dir.display());
			return;
		}

		for path in paths {
			let start = std::time::Instant::now();
			let mut source = from_file(&path)
				.unwrap_or_else(|e| panic!("could not open {}: {}", path.display(), e));
			let open_time = start.elapsed();

			// Timestamps are already corrected at this point, but the scan over all entries is the same
			let start = std::time::Instant::now();
			glog::adjust_sensor_timestamps(&mut source);
			let adjust_time = start.elapsed();

			let entries = count_entries(&source);
			println!(
				"{}: entries: {}, open: {:?}, adjust_sensor_timestamps: {:?}",
				path.file_name().unwrap().to_string_lossy(),
				entries,
				open_time,
				adjust_time
			);
			assert!(entries > 0, "no log entries in {}", path.display());
		}
	}
}

// Timestamp correction of sensor logs, from opening the file to the corrected entries.
// The test files are written to the temp directory and deleted afterwards.
#[cfg(test)]
mod time_correction_tests {
	use super::super::datetime_utils;
	use super::*;
	use std::io::Write;

	const T_BOOT: u64 = 621355968151283476; // 1970-01-01 00:00:15.128, before time sync
	const T_SYNCED: u64 = 639265188101410000; // 2026-10-02 06:20:10.141
	const DELTA_ETHERCAT: i64 = 1790921687981282700; // ns
	const DELTA_GPTP: i64 = 1672552120214243291; // ns

	fn entry(session_id: u32, ts: u64, message: &str) -> String {
		format!("[n|{}]:[t|{}]:[e|5420000]:[s|4]:[m|{}]\r\n", session_id, ts, message)
	}

	fn ethercat(session_id: u32, ts: u64, delta: i64) -> String {
		entry(session_id, ts, &format!("Setting EtherCAT time [delta = {} ns].", delta))
	}

	fn gptp(session_id: u32, ts: u64, delta: i64) -> String {
		entry(session_id, ts, &format!("Setting gPTP time [delta = {} ns]", delta))
	}

	fn raw(ts: u64) -> DateTime<Utc> {
		datetime_utils::from_100ns(ts).unwrap()
	}

	fn corrected(ts: u64, delta: i64) -> DateTime<Utc> {
		datetime_utils::add_offset_100ns(raw(ts), delta / 100).unwrap()
	}

	// A file in the temp directory that is deleted when dropped
	struct TempFile(PathBuf);

	impl Drop for TempFile {
		fn drop(&mut self) {
			let _ = std::fs::remove_file(&self.0);
		}
	}

	fn temp_path(name: &str) -> TempFile {
		TempFile(std::env::temp_dir().join(format!("sherlog_test_{}_{}", std::process::id(), name)))
	}

	fn write_sfile(name: &str, files: &[(&str, Vec<u8>)]) -> TempFile {
		let path = temp_path(&format!("{}.sfile", name));
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&path.0).unwrap());
		let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
		for (file_name, content) in files {
			zip.start_file(*file_name, options).unwrap();
			zip.write_all(content).unwrap();
		}
		zip.finish().unwrap();
		path
	}

	fn child<'a>(source: &'a model::LogSource, name: &str) -> &'a model::LogSource {
		match &source.children {
			model::LogSourceContents::Sources(v) => v.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no source {}", name)),
			model::LogSourceContents::Entries(_) => panic!("{} has no sub-sources", source.name),
		}
	}

	// (message, timestamp) of the entries of a sensor log source
	fn sensor_entries(root: &model::LogSource, board: &str, log: &str) -> Vec<(String, DateTime<Utc>)> {
		match &child(child(child(root, "Sensor"), board), log).children {
			model::LogSourceContents::Entries(v) => v.iter().map(|e| (e.message.clone(), e.timestamp)).collect(),
			model::LogSourceContents::Sources(_) => panic!("{} has sub-sources", log),
		}
	}

	fn open(name: &str, files: &[(&str, String)]) -> model::LogSource {
		let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, c)| (*n, c.clone().into_bytes())).collect();
		let sfile = write_sfile(name, &files);
		from_file(&sfile.0).unwrap()
	}

	#[test]
	fn ethercat_one_file() {
		let root = open(
			"ethercat_one_file",
			&[(
				"telescope_LoggerAtr_1_p.glog",
				entry(5, T_BOOT, "boot") + &ethercat(5, T_BOOT + 1, DELTA_ETHERCAT) + &entry(5, T_SYNCED, "synced"),
			)],
		);
		let entries = sensor_entries(&root, "telescope", "LoggerAtr_p");
		assert_eq!(entries[0], ("boot".to_string(), corrected(T_BOOT, DELTA_ETHERCAT)));
		assert_eq!(entries[2], ("synced".to_string(), raw(T_SYNCED)));
	}

	#[test]
	fn gptp_one_file() {
		let root = open(
			"gptp_one_file",
			&[(
				"telescope_LoggerAtr_1_p.glog",
				entry(5, T_BOOT, "boot") + &gptp(5, T_BOOT + 1, DELTA_GPTP) + &entry(5, T_SYNCED, "synced"),
			)],
		);
		let entries = sensor_entries(&root, "telescope", "LoggerAtr_p");
		assert_eq!(entries[0], ("boot".to_string(), corrected(T_BOOT, DELTA_GPTP)));
		assert_eq!(entries[2], ("synced".to_string(), raw(T_SYNCED)));
	}

	#[test]
	fn single_glog_file() {
		let path = temp_path("single_glog_file.glog");
		std::fs::write(&path.0, entry(5, T_BOOT, "boot") + &gptp(5, T_BOOT + 1, DELTA_GPTP)).unwrap();
		let source = glog::from_file(&path.0).unwrap();
		match &source.children {
			model::LogSourceContents::Entries(v) => assert_eq!(v[0].timestamp, corrected(T_BOOT, DELTA_GPTP)),
			model::LogSourceContents::Sources(_) => panic!("expected entries"),
		}
	}

	#[test]
	fn delta_in_newer_file_corrects_older_file() {
		// Ring buffer files: _2 is older than _1
		let root = open(
			"two_files",
			&[
				("telescope_LoggerAtr_1_p.glog", ethercat(5, T_BOOT + 2, DELTA_ETHERCAT) + &entry(5, T_SYNCED, "synced")),
				("telescope_LoggerAtr_2_p.glog", entry(5, T_BOOT, "boot 1") + &entry(5, T_BOOT + 1, "boot 2")),
			],
		);
		let entries = sensor_entries(&root, "telescope", "LoggerAtr_p");
		let messages: Vec<&str> = entries.iter().map(|(m, _)| m.as_str()).collect();
		assert_eq!(messages[0..2], ["boot 1", "boot 2"]);
		assert_eq!(messages[3], "synced");
		assert_eq!(entries[0].1, corrected(T_BOOT, DELTA_ETHERCAT));
		assert_eq!(entries[1].1, corrected(T_BOOT + 1, DELTA_ETHERCAT));
		assert_eq!(entries[3].1, raw(T_SYNCED));
	}

	#[test]
	fn multiple_sessions_across_two_files() {
		// Session 4: EtherCAT. Session 5: starts in the older file, gPTP delta in the newer file.
		// Session 6: never synchronized, ends with a power loss. Session 7: EtherCAT.
		let older = entry(4, T_BOOT, "session 4")
			+ &ethercat(4, T_BOOT + 1, DELTA_ETHERCAT)
			+ &entry(5, T_BOOT + 10, "session 5 in older file");
		let mut newer = entry(5, T_BOOT + 11, "session 5 in newer file")
			+ &gptp(5, T_BOOT + 12, DELTA_GPTP)
			+ &entry(6, T_BOOT + 20, "session 6");
		newer.push_str(&"\0".repeat(64));
		newer.push_str(&(entry(7, T_BOOT + 30, "session 7") + &ethercat(7, T_BOOT + 31, DELTA_ETHERCAT + 1_000_000_000)));
		let root = open(
			"multiple_sessions",
			&[("telescope_LoggerAtr_1_p.glog", newer), ("telescope_LoggerAtr_2_p.glog", older)],
		);
		let entries = sensor_entries(&root, "telescope", "LoggerAtr_p");
		let timestamp = |message: &str| entries.iter().find(|(m, _)| m == message).unwrap_or_else(|| panic!("no entry {}", message)).1;
		assert_eq!(timestamp("session 4"), corrected(T_BOOT, DELTA_ETHERCAT));
		assert_eq!(timestamp("session 5 in older file"), corrected(T_BOOT + 10, DELTA_GPTP));
		assert_eq!(timestamp("session 5 in newer file"), corrected(T_BOOT + 11, DELTA_GPTP));
		assert_eq!(timestamp("session 6"), raw(T_BOOT + 20));
		assert_eq!(timestamp("session 7"), corrected(T_BOOT + 30, DELTA_ETHERCAT + 1_000_000_000));
	}

	#[test]
	fn delta_does_not_correct_other_logs() {
		// Each log source is corrected with its own deltas, even with the same session ID
		let root = open(
			"other_logs",
			&[
				("telescope_LoggerAtr_1_p.glog", entry(5, T_BOOT, "atr") + &ethercat(5, T_BOOT + 1, DELTA_ETHERCAT)),
				("telescope_LoggerImaging_1_p.glog", entry(5, T_BOOT, "imaging")),
			],
		);
		assert_eq!(sensor_entries(&root, "telescope", "LoggerAtr_p")[0].1, corrected(T_BOOT, DELTA_ETHERCAT));
		assert_eq!(sensor_entries(&root, "telescope", "LoggerImaging_p")[0].1, raw(T_BOOT));
	}
}
