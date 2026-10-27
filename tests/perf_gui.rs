// Startup time of the GUI with generated sfiles, see src/startup_timing.rs.
// Needs a desktop session, Sherlog opens and closes its window three times per profile.
// Runs together with the other performance tests:
// cargo test --release perf -- --ignored --nocapture --test-threads=1
#[path = "../src/perf_history.rs"]
mod perf_history;
#[path = "../src/sfile_generator.rs"]
mod sfile_generator;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Starts Sherlog with the file and returns (window_ms, logs_ms)
fn start_sherlog(sfile: &std::path::Path) -> (u128, u128) {
	let result_file = std::env::temp_dir().join(format!("sherlog_startup_{}.txt", std::process::id()));
	let _ = std::fs::remove_file(&result_file);
	let t0 = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
	let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_sherlog"))
		.arg(sfile)
		.env("SHERLOG_STARTUP_TIMING", &result_file)
		.env("SHERLOG_STARTUP_T0_NS", t0.to_string())
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let deadline = Instant::now() + Duration::from_secs(120);
	while child.try_wait().unwrap().is_none() {
		if Instant::now() > deadline {
			let _ = child.kill();
			panic!("Sherlog did not show the log entries of {} within 120 s", sfile.display());
		}
		std::thread::sleep(Duration::from_millis(20));
	}
	let text = std::fs::read_to_string(&result_file).expect("Sherlog did not write the startup times");
	let _ = std::fs::remove_file(&result_file);
	let value = |name: &str| -> u128 {
		text.lines()
			.find_map(|line| line.strip_prefix(&format!("{}=", name)))
			.unwrap_or_else(|| panic!("no {} in {}", name, text))
			.parse()
			.unwrap()
	};
	(value("window_ms"), value("logs_ms"))
}

fn median(mut v: Vec<u128>) -> u128 {
	v.sort();
	v[v.len() / 2]
}

fn perf_gui(profile: &sfile_generator::Profile) {
	let (path, expected) = sfile_generator::cached(profile);
	let mut window = Vec::new();
	let mut logs = Vec::new();
	for _ in 0..3 {
		let (window_ms, logs_ms) = start_sherlog(&path);
		window.push(window_ms);
		logs.push(logs_ms);
	}
	let (window_ms, logs_ms) = (median(window), median(logs));
	println!("| {} | window {} ms | logs {} ms |", profile.name, window_ms, logs_ms);
	perf_history::record(
		profile.name,
		sfile_generator::GENERATOR_VERSION,
		expected.entries,
		&[("window_ms", window_ms), ("logs_ms", logs_ms)],
	);
}

#[test]
#[ignore]
fn perf_gui_1_typical_sfile() {
	perf_gui(&sfile_generator::TYPICAL);
}

#[test]
#[ignore]
fn perf_gui_2_large_sfile() {
	perf_gui(&sfile_generator::LARGE);
}
