// Tests with generated sfiles, see sfile_generator.rs.
// The performance tests are ignored by default. Run them before a release with:
// cargo test --release perf -- --ignored --nocapture --test-threads=1
// They compare with the last result of the same computer in NSIS/performance_history.csv.
// With SHERLOG_PERF_RECORD=1 set, they also add their result to that file.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::log_loader;
use super::log_store::SortMode;
use super::model;
use super::parse;
use super::sfile_generator::{self, Expected, Profile};

fn count_entries(source: &model::LogSource) -> usize {
	match &source.children {
		model::LogSourceContents::Sources(v) => v.iter().map(count_entries).sum(),
		model::LogSourceContents::Entries(v) => v.len(),
	}
}

fn count_before_2001(source: &model::LogSource) -> usize {
	let year_2001 = parse::datetime_utils::from_timestamp_ms(978_307_200_000).unwrap();
	match &source.children {
		model::LogSourceContents::Sources(v) => v.iter().map(count_before_2001).sum(),
		model::LogSourceContents::Entries(v) => v.iter().filter(|e| e.timestamp < year_2001).count(),
	}
}

fn sensor(root: &model::LogSource) -> &model::LogSource {
	match &root.children {
		model::LogSourceContents::Sources(v) => v.iter().find(|s| s.name == "Sensor").unwrap(),
		model::LogSourceContents::Entries(_) => panic!("no Sensor source"),
	}
}

fn check(path: &Path, expected: &Expected) -> model::LogSource {
	let root = parse::sfile::from_file(&path.to_path_buf()).unwrap();
	assert_eq!(count_entries(&root), expected.entries, "entries in {}", path.display());
	assert_eq!(count_before_2001(sensor(&root)), expected.sensor_entries_before_2001, "uncorrected sensor entries in {}", path.display());
	root
}

#[test]
fn generated_sfile_is_read_completely() {
	let path = std::env::temp_dir().join(format!("sherlog_test_{}_generated.sfile", std::process::id()));
	let expected = sfile_generator::generate(&path, &sfile_generator::SMALL).unwrap();
	let result = std::panic::catch_unwind(|| {
		check(&path, &expected);
	});
	let _ = std::fs::remove_file(&path);
	result.unwrap();
}

// Generates the file once, later runs reuse it. The expected counts are stored next to it.
fn cached_sfile(profile: &Profile) -> (PathBuf, Expected) {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("perf-data");
	std::fs::create_dir_all(&dir).unwrap();
	let base = format!("{}-v{}", profile.name, sfile_generator::GENERATOR_VERSION);
	let path = dir.join(format!("{}.sfile", base));
	let expected_path = dir.join(format!("{}.expected", base));
	if let Ok(text) = std::fs::read_to_string(&expected_path) {
		let numbers: Vec<usize> = text.split_whitespace().map(|n| n.parse().unwrap()).collect();
		if path.exists() && numbers.len() == 2 {
			return (path, Expected { entries: numbers[0], sensor_entries_before_2001: numbers[1] });
		}
	}
	println!("Generating {} ...", path.display());
	let expected = sfile_generator::generate(&path, profile).unwrap();
	std::fs::write(&expected_path, format!("{} {}", expected.entries, expected.sensor_entries_before_2001)).unwrap();
	(path, expected)
}

fn median(mut v: Vec<Duration>) -> Duration {
	v.sort();
	v[v.len() / 2]
}

const HISTORY_HEADER: &str = "date,version,commit,computer,profile,generator,entries,parse_ms,prepare_ms";

fn history_path() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("NSIS").join("performance_history.csv")
}

fn computer() -> String {
	std::env::var("COMPUTERNAME")
		.or_else(|_| std::env::var("HOSTNAME"))
		.unwrap_or_else(|_| "unknown".to_string())
}

// Short hash of HEAD, with "-dirty" if tracked files have changes
fn commit() -> String {
	let git = |args: &[&str]| {
		std::process::Command::new("git")
			.args(args)
			.current_dir(env!("CARGO_MANIFEST_DIR"))
			.output()
			.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
			.unwrap_or_default()
	};
	let hash = git(&["rev-parse", "--short", "HEAD"]);
	if git(&["status", "--porcelain", "--untracked-files=no"]).is_empty() {
		hash
	} else {
		hash + "-dirty"
	}
}

// Prints the difference to the last result of this computer, profile and generator version
fn compare_and_record(profile: &Profile, entries: usize, parse_ms: u128, prepare_ms: u128) {
	let generator = sfile_generator::GENERATOR_VERSION.to_string();
	let history = std::fs::read_to_string(history_path()).unwrap_or_default();
	let previous = history.lines().skip(1).filter(|line| {
		let fields: Vec<&str> = line.split(',').collect();
		fields.len() == 9 && fields[3] == computer() && fields[4] == profile.name && fields[5] == generator
	});
	match previous.last() {
		Some(line) => {
			let fields: Vec<&str> = line.split(',').collect();
			let change = |now: u128, before: &str| match before.parse::<f64>() {
				Ok(before) if before > 0.0 => format!("{} ms ({:+.0}%)", before, (now as f64 / before - 1.0) * 100.0),
				_ => "-".to_string(),
			};
			println!(
				"  previous: v{} {} ({}): parse {}, prepare {}",
				fields[1],
				fields[2],
				fields[0],
				change(parse_ms, fields[7]),
				change(prepare_ms, fields[8])
			);
		}
		None => println!("  previous: none for computer {}", computer()),
	}
	if std::env::var("SHERLOG_PERF_RECORD").as_deref() == Ok("1") {
		let mut text = if history.is_empty() { format!("{}\n", HISTORY_HEADER) } else { history };
		if !text.ends_with('\n') {
			text.push('\n');
		}
		text.push_str(&format!(
			"{},{},{},{},{},{},{},{},{}\n",
			chrono::Local::now().format("%Y-%m-%d"),
			env!("CARGO_PKG_VERSION"),
			commit(),
			computer(),
			profile.name,
			generator,
			entries,
			parse_ms,
			prepare_ms
		));
		std::fs::write(history_path(), text).unwrap();
		println!("  recorded in {}", history_path().display());
	}
}

fn perf(profile: &Profile) {
	let (path, expected) = cached_sfile(profile);
	let size_mb = std::fs::metadata(&path).unwrap().len() / 1_000_000;
	let mut parse_times = Vec::new();
	let mut prepare_times = Vec::new();
	for _ in 0..3 {
		let start = Instant::now();
		let root = parse::sfile::from_file(&path).unwrap();
		parse_times.push(start.elapsed());
		assert_eq!(count_entries(&root), expected.entries, "entries in {}", path.display());
		assert_eq!(count_before_2001(sensor(&root)), expected.sensor_entries_before_2001);
		let start = Instant::now();
		let loaded = log_loader::load(root, SortMode::Timestamp);
		prepare_times.push(start.elapsed());
		assert_eq!(loaded.entries.len(), expected.entries);
	}
	let parse_ms = median(parse_times).as_millis();
	let prepare_ms = median(prepare_times).as_millis();
	println!(
		"| {} | {} MB | {} entries | parse {} ms | prepare {} ms |",
		profile.name, size_mb, expected.entries, parse_ms, prepare_ms
	);
	compare_and_record(profile, expected.entries, parse_ms, prepare_ms);
}

#[test]
#[ignore]
fn perf_typical_sfile() {
	perf(&sfile_generator::TYPICAL);
}

#[test]
#[ignore]
fn perf_large_sfile() {
	perf(&sfile_generator::LARGE);
}
