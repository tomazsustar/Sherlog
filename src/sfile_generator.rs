// Writes synthetic sfiles for tests, so no real customer log files are needed.
// The content is deterministic: the same profile always gives the same file.
#![allow(dead_code)]

use std::io::Write;

// Part of the cache file names, increase it when the generated content changes
pub const GENERATOR_VERSION: u32 = 1;

pub struct Profile {
	pub name: &'static str,
	pub xlog_channels: usize,
	pub xlog_files_per_channel: usize,
	pub xlog_entries_per_file: usize,
	pub xlog_message_len: usize,
	pub huge_xlog_entries: usize, // One additional xlog file with long messages
	pub huge_xlog_message_len: usize,
	pub number_range: usize, // Numbers in messages are below this, smaller numbers compress better
	pub sensor_boards: usize,
	pub sensor_logs_per_board: usize,
	pub sensor_files_per_log: usize,
	pub sensor_entries_per_file: usize,
	pub controller_logs: usize,
	pub controller_files_per_log: usize,
	pub controller_entries_per_file: usize,
	pub rds_files: usize,
	pub rds_entries_per_file: usize,
}

// About as many entries as a typical sfile, many short xlog messages
pub const TYPICAL: Profile = Profile {
	name: "typical",
	xlog_channels: 40,
	xlog_files_per_channel: 10,
	xlog_entries_per_file: 3_400,
	xlog_message_len: 480,
	huge_xlog_entries: 0,
	huge_xlog_message_len: 0,
	number_range: 20,
	sensor_boards: 6,
	sensor_logs_per_board: 3,
	sensor_files_per_log: 3,
	sensor_entries_per_file: 7_000,
	controller_logs: 5,
	controller_files_per_log: 5,
	controller_entries_per_file: 2_000,
	rds_files: 5,
	rds_entries_per_file: 20_000,
};

// A large sfile, dominated by one xlog file of 500 MB
pub const LARGE: Profile = Profile {
	name: "large",
	xlog_channels: 20,
	xlog_files_per_channel: 5,
	xlog_entries_per_file: 2_000,
	xlog_message_len: 400,
	huge_xlog_entries: 100_000,
	huge_xlog_message_len: 5_000,
	number_range: 100_000,
	sensor_boards: 6,
	sensor_logs_per_board: 3,
	sensor_files_per_log: 2,
	sensor_entries_per_file: 2_000,
	controller_logs: 5,
	controller_files_per_log: 5,
	controller_entries_per_file: 2_000,
	rds_files: 17,
	rds_entries_per_file: 40_000,
};

// Small enough for a unit test
pub const SMALL: Profile = Profile {
	name: "small",
	xlog_channels: 3,
	xlog_files_per_channel: 2,
	xlog_entries_per_file: 50,
	xlog_message_len: 100,
	huge_xlog_entries: 20,
	huge_xlog_message_len: 2_000,
	number_range: 1_000,
	sensor_boards: 2,
	sensor_logs_per_board: 2,
	sensor_files_per_log: 3,
	sensor_entries_per_file: 40,
	controller_logs: 2,
	controller_files_per_log: 3,
	controller_entries_per_file: 30,
	rds_files: 2,
	rds_entries_per_file: 50,
};

pub const PROFILES: [&Profile; 3] = [&SMALL, &TYPICAL, &LARGE];

// What Sherlog must read from a generated file
#[derive(Debug, Default, PartialEq)]
pub struct Expected {
	pub entries: usize,
	// Sensor entries that keep their 1970 timestamps: sessions without a time delta, and the delta entries
	pub sensor_entries_before_2001: usize,
}

const BOARDS: [&str; 6] = ["telescope", "axis", "trigger", "laserctl", "wlanmodule", "cfmcw"];
const SENSOR_LOGS: [&str; 3] = ["LoggerAtr", "MainLogFLASH", "LoggerImaging"];
const CONTROLLER_LOGS: [&str; 5] = ["TrackerLucent", "ProcessManager", "IOT", "Axis", "CFMCW"];
const CONTROLLER_SOURCES: [&str; 4] = ["AxisSrvFsmDevice", "ProcessManager", "IotClient", "DeviceManager"];
const WORDS: [&str; 16] = [
	"System", "Mode", "changed", "to", "Operating", "measurement", "started", "target", "found", "ATR", "spot",
	"position", "update", "successful", "command", "received",
];
const XLOG_SEVERITIES: [&str; 5] = ["Info", "Info", "Info", "Warning", "Error"];
const RDS_SEVERITIES: [&str; 5] = ["info", "info", "warning", "error", "debug"];

const T_SYNCED: u64 = 639265188101410000; // 2026-10-02 06:20:10, in 100 ns since 0001-01-01
const T_BOOT: u64 = 621355968151283476; // 1970-01-01 00:00:15, before time sync
const DELTA: i64 = 1790921687981282700; // ns

// xorshift64*, enough for test data
struct Random(u64);

impl Random {
	fn new(seed: &str) -> Random {
		let mut state = 0x9E3779B97F4A7C15u64;
		for b in seed.bytes() {
			state = (state ^ b as u64).wrapping_mul(0x100000001B3);
		}
		Random(state | 1)
	}

	fn next(&mut self) -> u64 {
		self.0 ^= self.0 >> 12;
		self.0 ^= self.0 << 25;
		self.0 ^= self.0 >> 27;
		self.0.wrapping_mul(0x2545F4914F6CDD1D)
	}

	fn below(&mut self, n: usize) -> usize {
		(self.next() % n as u64) as usize
	}
}

// A message of about `len` bytes: words, and numbers like in measurement dumps
fn message(random: &mut Random, len: usize, number_range: usize) -> String {
	let mut s = String::with_capacity(len + 16);
	while s.len() < len {
		if random.below(3) == 0 {
			s.push_str(&format!("{}.{:02};", random.below(number_range), random.below(number_range.min(100))));
		} else {
			s.push_str(WORDS[random.below(WORDS.len())]);
			s.push(' ');
		}
	}
	s
}

fn xlog(random: &mut Random, channel: &str, entries: usize, message_len: usize, number_range: usize, mut t: u64) -> Vec<u8> {
	let mut out = Vec::new();
	for _ in 0..entries {
		t += 1 + random.below(10_000_000) as u64;
		write!(
			out,
			"<T>˩{}˫<A>˩TrackerPilot˫<I>˩4711˫<C>˩{}˫<L>˩{}˫<M>˩{}\r\n",
			t,
			channel,
			XLOG_SEVERITIES[random.below(XLOG_SEVERITIES.len())],
			message(random, message_len, number_range)
		)
		.unwrap();
	}
	out
}

// Ring buffer files of one sensor log, oldest first. Two sessions per file: each session starts
// before time sync. Every third session never gets a time delta and ends with flash padding.
fn sensor_files(random: &mut Random, profile: &Profile, expected: &mut Expected) -> Vec<Vec<u8>> {
	let mut files = Vec::new();
	let mut session = 100;
	for _ in 0..profile.sensor_files_per_log {
		let mut out = Vec::new();
		for _ in 0..2 {
			session += 1;
			let synced = session % 3 != 0;
			let per_session = profile.sensor_entries_per_file / 2;
			let boot_entries = per_session / 4;
			let mut t = T_BOOT;
			for i in 0..per_session {
				let (ts, text) = if i < boot_entries || !synced {
					t += 1 + random.below(100_000) as u64;
					(t, message(random, 60, profile.number_range))
				} else if i == boot_entries {
					let delta = if session % 2 == 0 { DELTA } else { DELTA + 1_000_000_000 };
					let source = if session % 2 == 0 { "EtherCAT" } else { "gPTP" };
					(t + 1, format!("Setting {} time [delta = {} ns]", source, delta))
				} else {
					t += 1 + random.below(100_000) as u64;
					(T_SYNCED + (t - T_BOOT), message(random, 60, profile.number_range))
				};
				write!(out, "[n|{}]:[t|{}]:[e|5420000]:[s|{}]:[m|{}]\r\n", session, ts, 3 + random.below(2), text).unwrap();
				expected.entries += 1;
				// The time delta entry itself keeps its timestamp
				if !synced || i == boot_entries {
					expected.sensor_entries_before_2001 += 1;
				}
			}
			if !synced {
				out.extend(std::iter::repeat(0u8).take(209));
			}
		}
		files.push(out);
	}
	files
}

// Ring buffer files of one controller log, oldest first
fn controller_files(random: &mut Random, profile: &Profile, expected: &mut Expected) -> Vec<Vec<u8>> {
	let mut files = Vec::new();
	let mut t_ms: u64 = 1_790_853_428_348;
	for _ in 0..profile.controller_files_per_log {
		let mut out = Vec::new();
		for _ in 0..profile.controller_entries_per_file {
			t_ms += 1 + random.below(1_000) as u64;
			let source = CONTROLLER_SOURCES[random.below(CONTROLLER_SOURCES.len())];
			write!(out, "[tq|{:018}]:[s|{}]:[i|13]:[m|{}: {}]\n", t_ms, 2 + random.below(3), source, message(random, 80, profile.number_range)).unwrap();
			expected.entries += 1;
		}
		files.push(out);
	}
	files
}

fn rds(random: &mut Random, entries: usize, number_range: usize) -> Vec<u8> {
	let mut out = Vec::new();
	for i in 0..entries {
		write!(
			out,
			"2026-03-05 12:{:02}:{:02}.{:03}|36484|{}|apinet|{}\r\n",
			(i / 60_000) % 60,
			(i / 1_000) % 60,
			i % 1_000,
			RDS_SEVERITIES[random.below(RDS_SEVERITIES.len())],
			message(random, 100, number_range)
		)
		.unwrap();
	}
	out
}

fn add<W: Write + std::io::Seek>(zip: &mut zip::ZipWriter<W>, name: &str, content: &[u8]) -> std::io::Result<()> {
	let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
	zip.start_file(name, options)?;
	zip.write_all(content)
}

pub fn generate(path: &std::path::Path, profile: &Profile) -> std::io::Result<Expected> {
	let mut expected = Expected::default();
	let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(path)?));

	for c in 0..profile.xlog_channels {
		let channel = format!("LMF.Channel{}", c);
		for f in 0..profile.xlog_files_per_channel {
			let mut random = Random::new(&format!("xlog {} {}", c, f));
			let content = xlog(&mut random, &channel, profile.xlog_entries_per_file, profile.xlog_message_len, profile.number_range, T_SYNCED + f as u64 * 1_000_000_000);
			expected.entries += profile.xlog_entries_per_file;
			add(&mut zip, &format!("TrackerPilot_4711_{}_2026-10-02-06-{:02}-00-0000.xlog", channel, f), &content)?;
		}
	}
	if profile.huge_xlog_entries > 0 {
		let mut random = Random::new("huge xlog");
		let content = xlog(&mut random, "LMF.MeasurementWithWarnings", profile.huge_xlog_entries, profile.huge_xlog_message_len, profile.number_range, T_SYNCED);
		expected.entries += profile.huge_xlog_entries;
		add(&mut zip, "TrackerPilot_4711_LMF.MeasurementWithWarnings_2026-10-01-00-00-00-0000.xlog", &content)?;
	}

	for board in BOARDS.iter().take(profile.sensor_boards) {
		for log in SENSOR_LOGS.iter().take(profile.sensor_logs_per_board) {
			let mut random = Random::new(&format!("sensor {} {}", board, log));
			let files = sensor_files(&mut random, profile, &mut expected);
			let count = files.len();
			for (i, content) in files.iter().enumerate() {
				// The highest ring buffer number is the oldest file
				add(&mut zip, &format!("{}_{}_{}_p.glog", board, log, count - i), content)?;
			}
		}
	}
	{
		// Sensor log without session IDs
		let mut content = Vec::new();
		for i in 0..profile.sensor_entries_per_file {
			write!(content, "[t|{}]:[e|0]:[s|4]:[m|T= {}.{} degC; H= 30.0 %relH]\r\n", T_SYNCED + i as u64, 15 + i % 10, i % 10).unwrap();
			expected.entries += 1;
		}
		add(&mut zip, "sensorbase_BaseboardSpecialLogs_1_v.glog", &content)?;
	}

	for log in CONTROLLER_LOGS.iter().take(profile.controller_logs) {
		let mut random = Random::new(&format!("controller {}", log));
		let files = controller_files(&mut random, profile, &mut expected);
		let count = files.len();
		for (i, content) in files.iter().enumerate() {
			// Oldest first: contr_X_4, ..., contr_X_1, then contr_X
			let name = if i + 1 == count { format!("contr_{}.glog", log) } else { format!("contr_{}_{}.glog", log, count - 1 - i) };
			add(&mut zip, &name, content)?;
		}
	}

	for r in 0..profile.rds_files {
		let mut random = Random::new(&format!("rds {}", r));
		add(&mut zip, &format!("RDS/Hexagon.Rds.Service{}.log", r), &rds(&mut random, profile.rds_entries_per_file, profile.number_range))?;
		expected.entries += profile.rds_entries_per_file;
	}

	// Files that are not log files
	add(&mut zip, "supportinfo.xml", b"<supportinfo generated=\"true\"/>")?;
	add(&mut zip, "compensations.xml", message(&mut Random::new("xml"), 40_000, 100).as_bytes())?;

	zip.finish()?.flush()?;
	Ok(expected)
}

// Generates the file into target/perf-data once, later calls reuse it. The expected counts are stored next to it.
pub fn cached(profile: &Profile) -> (std::path::PathBuf, Expected) {
	let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("perf-data");
	std::fs::create_dir_all(&dir).unwrap();
	let base = format!("{}-v{}", profile.name, GENERATOR_VERSION);
	let path = dir.join(format!("{}.sfile", base));
	let expected_path = dir.join(format!("{}.expected", base));
	if let Ok(text) = std::fs::read_to_string(&expected_path) {
		let numbers: Vec<usize> = text.split_whitespace().map(|n| n.parse().unwrap()).collect();
		if path.exists() && numbers.len() == 2 {
			return (path, Expected { entries: numbers[0], sensor_entries_before_2001: numbers[1] });
		}
	}
	println!("Generating {} ...", path.display());
	let expected = generate(&path, profile).unwrap();
	std::fs::write(&expected_path, format!("{} {}", expected.entries, expected.sensor_entries_before_2001)).unwrap();
	(path, expected)
}
