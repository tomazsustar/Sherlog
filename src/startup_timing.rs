// Startup time measurement for the performance tests in tests/perf_gui.rs.
// Enabled by SHERLOG_STARTUP_TIMING=<result file>. Times are measured from SHERLOG_STARTUP_T0_NS
// (nanoseconds since the Unix epoch, set by the test before it starts Sherlog), or else from main().
// A frame counts when GTK has painted it, after the draw handler.
// Sherlog writes the result file after the first painted frame with log entries, and exits shortly after.
use gtk::glib;
use gtk::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Timing {
	file: String,
	t0: SystemTime,
	window_ms: OnceLock<u128>,
	logs_drawn: AtomicBool,
	connected: AtomicBool,
	done: AtomicBool,
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
		Some(Timing {
			file,
			t0,
			window_ms: OnceLock::new(),
			logs_drawn: AtomicBool::new(false),
			connected: AtomicBool::new(false),
			done: AtomicBool::new(false),
		})
	});
}

pub fn enabled() -> bool {
	matches!(TIMING.get(), Some(Some(_)))
}

// Called by the draw handler of the log view
pub fn frame_drawn(widget: &impl IsA<gtk::Widget>, logs_visible: bool) {
	let Some(Some(timing)) = TIMING.get() else {
		return;
	};
	if logs_visible {
		timing.logs_drawn.store(true, Ordering::SeqCst);
	}
	if timing.connected.swap(true, Ordering::SeqCst) {
		return;
	}
	if let Some(frame_clock) = widget.frame_clock() {
		frame_clock.connect_after_paint(|_| after_paint());
	}
}

fn after_paint() {
	let Some(Some(timing)) = TIMING.get() else {
		return;
	};
	let ms = timing.t0.elapsed().map(|d| d.as_millis()).unwrap_or(0);
	let window_ms = *timing.window_ms.get_or_init(|| ms);
	if timing.logs_drawn.load(Ordering::SeqCst) && !timing.done.swap(true, Ordering::SeqCst) {
		std::fs::write(&timing.file, format!("window_ms={}\nlogs_ms={}\n", window_ms, ms)).unwrap();
		// Leave the window open for a moment, so it can be seen
		glib::timeout_add_local_once(Duration::from_millis(500), || std::process::exit(0));
	}
}
