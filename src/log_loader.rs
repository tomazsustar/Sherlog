use std::collections::HashMap;
use std::time::Instant;

use super::log_store::{LogStoreLinear, SortMode};
use super::model;
use super::model_internal::{LogEntryExt, LogSourceContentsExt, LogSourceExt};
use super::parse;

// A log file prepared for the log view. Contains no GTK objects, so it can be created in any thread.
pub struct LoadedLog {
	pub root: LogSourceExt,                // Tree of log sources, without their entries
	pub entries: Vec<LogEntryExt>,         // Entries of all log sources, sorted
	pub log_sources: HashMap<u32, String>, // Path of each log source, by source ID
	pub log_sources_to_shift: Vec<u32>,    // Log sources that the sensor time shift applies to
}

pub fn load_file(path: &std::path::PathBuf, sort_mode: SortMode) -> Result<LoadedLog, parse::io::LogParseError> {
	let now = Instant::now();
	let root = parse::from_file(path)?;
	log::info!("Time to parse file: {}ms", now.elapsed().as_millis());
	Ok(load(root, sort_mode))
}

pub fn load(root: model::LogSource, sort_mode: SortMode) -> LoadedLog {
	let now = Instant::now();
	let mut root = LogSourceExt::from_source(root);

	let mut entries = Vec::new();
	move_entries(&mut root, &mut entries);
	entries.sort_by(|a, b| LogStoreLinear::compare_entries(sort_mode, a, b));

	let mut log_sources = HashMap::new();
	collect_log_sources(&root, String::new(), &mut log_sources);

	let mut log_sources_to_shift = Vec::new();
	root.collect_descendant_ids_of_filtered_roots(
		&|src| matches!(src.name.as_str(), "Controller" | "Sensor" | "Probe" | "Connect Box"),
		&mut log_sources_to_shift,
	);

	log::info!("Time to create store: {}ms", now.elapsed().as_millis());
	LoadedLog {
		root,
		entries,
		log_sources,
		log_sources_to_shift,
	}
}

fn move_entries(log_source: &mut LogSourceExt, entries: &mut Vec<LogEntryExt>) {
	match &mut log_source.children {
		LogSourceContentsExt::Sources(v) => {
			for source in v {
				move_entries(source, entries);
			}
		}
		LogSourceContentsExt::Entries(v) => {
			entries.append(v);
		}
	}
}

fn collect_log_sources(log_source: &LogSourceExt, prefix: String, log_sources: &mut HashMap<u32, String>) {
	let current_name = prefix + "/" + &log_source.name;
	log_sources.insert(log_source.id, current_name.clone());
	if let LogSourceContentsExt::Sources(v) = &log_source.children {
		for source in v {
			collect_log_sources(source, current_name.clone(), log_sources);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn entry(seconds: u64, message: &str) -> model::LogEntry {
		model::LogEntry {
			timestamp: parse::datetime_utils::from_timestamp_ms(seconds * 1000).unwrap(),
			message: message.to_string(),
			..Default::default()
		}
	}

	fn source(name: &str, children: model::LogSourceContents) -> model::LogSource {
		model::LogSource { name: name.to_string(), children }
	}

	#[test]
	fn load_sorts_entries_and_collects_sources() {
		let root = source(
			"root",
			model::LogSourceContents::Sources(vec![
				source(
					"Sensor",
					model::LogSourceContents::Sources(vec![source(
						"axis",
						model::LogSourceContents::Entries(vec![entry(3, "third"), entry(1, "first")]),
					)]),
				),
				source("Client", model::LogSourceContents::Entries(vec![entry(2, "second")])),
			]),
		);
		let loaded = load(root, SortMode::Timestamp);

		let messages: Vec<&str> = loaded.entries.iter().map(|e| e.message.as_str()).collect();
		assert_eq!(messages, vec!["first", "second", "third"]);

		let id = |path: &str| *loaded.log_sources.iter().find(|(_, p)| p.as_str() == path).unwrap().0;
		let mut shift = loaded.log_sources_to_shift.clone();
		shift.sort();
		let mut expected = vec![id("/root/Sensor"), id("/root/Sensor/axis")];
		expected.sort();
		assert_eq!(shift, expected);
		assert_eq!(loaded.log_sources.len(), 4);

		// The entries were moved out of the tree
		match &loaded.root.children {
			LogSourceContentsExt::Sources(v) => match &v[1].children {
				LogSourceContentsExt::Entries(e) => assert!(e.is_empty()),
				_ => panic!("expected entries"),
			},
			_ => panic!("expected sources"),
		}
	}
}
