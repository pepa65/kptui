use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::util::expand_tilde;

const SAMPLE_CONFIG: &str = include_str!("../sample-config.toml");
const AUTO_LOCK: u64 = 300;
const CONFIGFILE: &str = "~/.config/kptui/config.toml";

pub struct Config {
	pub default_database: Option<PathBuf>,
	pub keyfile: Option<PathBuf>,
	pub auto_lock: Duration,
	pub theme: Option<String>,
}

impl Default for Config {
	fn default() -> Self {
		Self { default_database: None, keyfile: None, auto_lock: Duration::from_secs(AUTO_LOCK), theme: None }
	}
}

#[derive(Deserialize, Default)]
struct ConfigFile {
	default_database: Option<String>,
	keyfile: Option<String>,
	auto_lock: Option<u64>,
	theme: Option<String>,
}

pub fn load_config() -> anyhow::Result<Config> {
	let path = expand_tilde(CONFIGFILE);

	if !path.is_file() {
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
		}

		fs::write(&path, SAMPLE_CONFIG).with_context(|| format!("couldn't write {}", path.display()))?;
	}

	let text = fs::read_to_string(&path).with_context(|| format!("couldn't read {}", path.display()))?;
	parse_config(&text)
}

fn parse_config(text: &str) -> anyhow::Result<Config> {
	let raw: ConfigFile = toml::from_str(text)?;

	let defaults = Config::default();

	Ok(Config {
		default_database: raw.default_database.map(|s| expand_tilde(&s)),
		keyfile: raw.keyfile.map(|s| expand_tilde(&s)),
		auto_lock: raw.auto_lock.map(Duration::from_secs).unwrap_or(defaults.auto_lock),
		theme: raw.theme,
	})
}

#[derive(Serialize)]
struct ConfigFileOut<'a> {
	default_database: Option<String>,
	keyfile: Option<String>,
	auto_lock: u64,
	theme: Option<&'a str>,
}

pub fn save_config(config: &Config) -> anyhow::Result<()> {
	let path = expand_tilde(CONFIGFILE);

	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
	}

	let text = serialize_config(config)?;

	fs::write(&path, text).with_context(|| format!("couldn't write {}", path.display()))?;

	Ok(())
}

fn serialize_config(config: &Config) -> anyhow::Result<String> {
	let out = ConfigFileOut {
		default_database: config.default_database.as_ref().map(|p| p.display().to_string()),
		keyfile: config.keyfile.as_ref().map(|p| p.display().to_string()),
		auto_lock: config.auto_lock.as_secs(),
		theme: config.theme.as_deref(),
	};

	toml::to_string_pretty(&out).context("couldn't serialize config")
}

#[cfg(test)]
mod tests {
	use super::{Config, parse_config, serialize_config};
	use std::path::PathBuf;

	#[test]
	fn parses_optional_keyfile_path() {
		let config = parse_config(
			r#"
                default_database = "/tmp/passwords.kdbx"
                keyfile = "/tmp/passwords.keyx"
            "#,
		)
		.expect("config should parse");

		assert_eq!(config.default_database, Some(PathBuf::from("/tmp/passwords.kdbx")));
		assert_eq!(config.keyfile, Some(PathBuf::from("/tmp/passwords.keyx")));
	}

	#[test]
	fn keyfile_remains_optional() {
		let config = parse_config("default_database = \"/tmp/passwords.kdbx\"").expect("config should parse");

		assert_eq!(config.keyfile, None);

		let text = serialize_config(&config).expect("config should serialize");
		assert!(!text.contains("keyfile"));
	}

	#[test]
	fn serializes_keyfile_path() {
		let config = Config { keyfile: Some(PathBuf::from("/tmp/passwords.keyx")), ..Config::default() };

		let text = serialize_config(&config).expect("config should serialize");
		assert!(text.contains("keyfile = \"/tmp/passwords.keyx\""));
	}
}
