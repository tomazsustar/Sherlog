// Tests with generated sfiles, see sfile_generator.rs.
// The performance tests are ignored by default. Run them before a release with:
// cargo test --release perf -- --ignored --nocapture --test-threads=1
// This also runs the GUI performance tests in tests/perf_gui.rs.
// They compare with the last result of the same computer in NSIS/performance_history.csv.
// With SHERLOG_PERF_RECORD=1 set, they also add their result to that file.
use std::path::Path;
use std::time::{Duration, Instant};

use super::log_loader;
use super::log_store::SortMode;
use super::model;
use super::parse;
use super::perf_history;
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

fn median(mut v: Vec<Duration>) -> Duration {
	v.sort();
	v[v.len() / 2]
}

fn perf(profile: &Profile) {
	let (path, expected) = sfile_generator::cached(profile);
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
	perf_history::record(profile.name, sfile_generator::GENERATOR_VERSION, expected.entries, &[("parse_ms", parse_ms), ("prepare_ms", prepare_ms)]);
}

#[test]
#[ignore]
fn perf_1_typical_sfile() {
	perf(&sfile_generator::TYPICAL);
}

#[test]
#[ignore]
fn perf_2_large_sfile() {
	perf(&sfile_generator::LARGE);
}
