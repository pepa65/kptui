use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use serde::Serialize;

use crate::db::Entry;

pub enum ExportFormat {
	Kdbx,
	Csv,
	Json,
}

pub fn detect_format(path: &Path) -> anyhow::Result<ExportFormat> {
	let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());

	match ext.as_deref() {
		Some("kdbx") => Ok(ExportFormat::Kdbx),
		Some("csv") => Ok(ExportFormat::Csv),
		Some("json") => Ok(ExportFormat::Json),
		_ => anyhow::bail!("unrecognized file extension, expected .kdbx, .csv, or .json"),
	}
}

pub fn export_csv(path: &Path, entries: &[Entry]) -> anyhow::Result<()> {
	let mut options = OpenOptions::new();
	options.write(true).create_new(true);

	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}

	let file = options.open(path).with_context(|| format!("couldn't create {}", path.display()))?;

	let mut writer = csv::WriterBuilder::new().from_writer(file);

	writer.write_record(["name", "username", "password", "url", "totp", "notes"])?;

	for entry in entries {
		writer.write_record([
			entry.name.as_str(),
			entry.user.as_str(),
			entry.password.as_str(),
			entry.url.as_str(),
			entry.totp.as_str(),
			entry.notes.as_str(),
		])?;
	}

	writer.flush().context("couldn't finish writing CSV")?;

	Ok(())
}

#[derive(Serialize)]
struct ExportEntry<'a> {
	name: &'a str,
	username: &'a str,
	password: &'a str,
	url: &'a str,
	totp: &'a str,
	notes: &'a str,
}

pub fn export_json(path: &Path, entries: &[Entry]) -> anyhow::Result<()> {
	let out: Vec<ExportEntry> = entries
		.iter()
		.map(|e| ExportEntry { name: &e.name, username: &e.user, password: &e.password, url: &e.url, totp: &e.totp, notes: &e.notes })
		.collect();

	let json = serde_json::to_string_pretty(&out).context("couldn't serialize entries")?;

	let mut options = OpenOptions::new();
	options.write(true).create_new(true);

	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}

	let mut file = options.open(path).with_context(|| format!("couldn't create {}", path.display()))?;

	file.write_all(json.as_bytes()).with_context(|| format!("couldn't write {}", path.display()))?;

	Ok(())
}
