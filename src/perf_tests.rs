// Tests with generated sfiles, see sfile_generator.rs.
// The performance tests are ignored by default. Run them before a release with:
// cargo test --release perf -- --ignored --nocapture --test-threads=1
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

fn perf(profile: &Profile) {
	let (path, expected) = cached_sfile(profile);
	let size_mb = std::fs::metadata(&path).unwrap().len() / 1_000_000;
	let mut parse_times = Vec::new();
	let mut prepare_times = Vec::new();
	for _ in 0..3 {
		let start = Instant::now();
		let root = check(&path, &expected);
		parse_times.push(start.elapsed());
		let start = Instant::now();
		let loaded = log_loader::load(root, SortMode::Timestamp);
		prepare_times.push(start.elapsed());
		assert_eq!(loaded.entries.len(), expected.entries);
	}
	println!(
		"| {} | {} MB | {} entries | parse {} ms | prepare {} ms |",
		profile.name,
		size_mb,
		expected.entries,
		median(parse_times).as_millis(),
		median(prepare_times).as_millis()
	);
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
