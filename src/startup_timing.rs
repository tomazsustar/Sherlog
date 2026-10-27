// Startup time measurement for the performance tests in tests/perf_gui.rs.
// Enabled by SHERLOG_STARTUP_TIMING=<result file>. Times are measured from SHERLOG_STARTUP_T0_NS
// (nanoseconds since the Unix epoch, set by the test before it starts Sherlog), or else from main().
// Sherlog writes the result file and exits when the first frame with log entries is drawn.
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Timing {
	file: String,
	t0: SystemTime,
	window_ms: OnceLock<u128>,
}

static TIMING: OnceLock<Option<Timing>> = OnceLock::new();

pub fn init() {
	TIMING.get_or_init(|| {
		let file = std::env::var("SHERLOG_STARTUP_TIMING").ok()?;
		let t0 = std::env::var("SHERLOG_STARTUP_T0_NS")
			.ok()
			.and_then(|ns| ns.parse::<u64>().ok())
			.map(|ns| UNIX_EPOCH + Duration::from_nanos(ns))
			.unwrap_or_else(SystemTime::now);
		Some(Timing { file, t0, window_ms: OnceLock::new() })
	});
}

pub fn enabled() -> bool {
	matches!(TIMING.get(), Some(Some(_)))
}

pub fn frame_drawn(logs_visible: bool) {
	if let Some(Some(timing)) = TIMING.get() {
		let ms = timing.t0.elapsed().map(|d| d.as_millis()).unwrap_or(0);
		let window_ms = *timing.window_ms.get_or_init(|| ms);
		if logs_visible {
			std::fs::write(&timing.file, format!("window_ms={}\nlogs_ms={}\n", window_ms, ms)).unwrap();
			std::process::exit(0);
		}
	}
}
