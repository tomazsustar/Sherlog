// Results of the performance tests in NSIS/performance_history.csv, one row per release and profile.
// Used by src/perf_tests.rs and tests/perf_gui.rs, which fill in different columns of the same row.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const COLUMNS: [&str; 11] = [
	"date", "version", "commit", "computer", "profile", "generator", "entries", "parse_ms", "prepare_ms", "window_ms", "logs_ms",
];

fn column(name: &str) -> usize {
	COLUMNS.iter().position(|c| *c == name).unwrap()
}

pub fn path() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("NSIS").join("performance_history.csv")
}

pub fn computer() -> String {
	std::env::var("COMPUTERNAME")
		.or_else(|_| std::env::var("HOSTNAME"))
		.unwrap_or_else(|_| "unknown".to_string())
}

// Short hash of HEAD, with "-dirty" if tracked files other than the results have changes
pub fn commit() -> String {
	let git = |args: &[&str]| {
		std::process::Command::new("git")
			.args(args)
			.current_dir(env!("CARGO_MANIFEST_DIR"))
			.output()
			.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
			.unwrap_or_default()
	};
	let hash = git(&["rev-parse", "--short", "HEAD"]);
	if git(&["status", "--porcelain", "--untracked-files=no", "--", ".", ":!NSIS/performance_history.csv"]).is_empty() {
		hash
	} else {
		hash + "-dirty"
	}
}

fn read_rows() -> Vec<Vec<String>> {
	let text = std::fs::read_to_string(path()).unwrap_or_default();
	text.lines()
		.skip(1)
		.filter(|line| !line.is_empty())
		.map(|line| {
			let mut fields: Vec<String> = line.split(',').map(|f| f.to_string()).collect();
			fields.resize(COLUMNS.len(), String::new());
			fields
		})
		.collect()
}

// Prints the change to the last result of this computer, profile and generator from another commit.
// With SHERLOG_PERF_RECORD=1, also stores the results.
pub fn record(profile: &str, generator: u32, entries: usize, results: &[(&str, u128)]) {
	let mut rows = read_rows();
	let (commit, computer, generator) = (commit(), computer(), generator.to_string());
	let same_setup = |row: &Vec<String>| row[column("computer")] == computer && row[column("profile")] == profile && row[column("generator")] == generator;

	for (name, value) in results {
		let previous = rows.iter().rev().find(|row| same_setup(row) && row[column("commit")] != commit && !row[column(name)].is_empty());
		match previous {
			Some(row) => {
				let before: f64 = row[column(name)].parse().unwrap_or(0.0);
				let change = if before > 0.0 { (*value as f64 / before - 1.0) * 100.0 } else { 0.0 };
				println!(
					"  {}: {} ms, v{} {} had {} ms ({:+.0}%)",
					name,
					value,
					row[column("version")],
					row[column("commit")],
					before,
					change
				);
			}
			None => println!("  {}: {} ms, no earlier result on computer {}", name, value, computer),
		}
	}

	if std::env::var("SHERLOG_PERF_RECORD").as_deref() != Ok("1") {
		return;
	}
	let version = env!("CARGO_PKG_VERSION");
	let empty = |row: &Vec<String>| results.iter().all(|(name, _)| row[column(name)].is_empty());
	let index = match rows.iter().rposition(|row| same_setup(row) && row[column("commit")] == commit && row[column("version")] == version) {
		Some(i) if empty(&rows[i]) => i,
		_ => {
			let mut row = vec![String::new(); COLUMNS.len()];
			row[column("date")] = chrono::Local::now().format("%Y-%m-%d").to_string();
			row[column("version")] = version.to_string();
			row[column("commit")] = commit.clone();
			row[column("computer")] = computer.clone();
			row[column("profile")] = profile.to_string();
			row[column("generator")] = generator.clone();
			rows.push(row);
			rows.len() - 1
		}
	};
	rows[index][column("entries")] = entries.to_string();
	for (name, value) in results {
		rows[index][column(name)] = value.to_string();
	}
	let mut text = COLUMNS.join(",") + "\n";
	for row in &rows {
		text.push_str(&row.join(","));
		text.push('\n');
	}
	std::fs::write(path(), text).unwrap();
	println!("  recorded in {}", path().display());
}
