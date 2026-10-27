// Writes a synthetic sfile to open in Sherlog, without real customer log files.
// Usage: cargo run --release --example generate_sfile -- <small|typical|large> <output.sfile>
#[path = "../src/sfile_generator.rs"]
mod sfile_generator;

fn main() {
	let args: Vec<String> = std::env::args().collect();
	let profile = args.get(1).and_then(|name| sfile_generator::PROFILES.iter().find(|p| p.name == name));
	let (profile, output) = match (profile, args.get(2)) {
		(Some(profile), Some(output)) => (profile, output),
		_ => {
			let names: Vec<&str> = sfile_generator::PROFILES.iter().map(|p| p.name).collect();
			eprintln!("Usage: generate_sfile <{}> <output.sfile>", names.join("|"));
			std::process::exit(1);
		}
	};
	let expected = sfile_generator::generate(std::path::Path::new(output), profile).unwrap();
	println!("Wrote {} with {} log entries", output, expected.entries);
}
