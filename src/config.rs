use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::util::expand_tilde;

const SAMPLE_CONFIG: &str = include_str!("../default_config.toml");
const AUTO_EXIT: u64 = 300;
const CONFIGFILE: &str = "~/.config/kptui/config.toml";
const KDF_ITERATIONS: u64 = 90;
const TMUX_CLEAR: u64 = 30;

fn sibling_tmp_path(path: &Path) -> PathBuf {
	let mut tmp = path.as_os_str().to_os_string();
	tmp.push(".tmp");
	PathBuf::from(tmp)
}

pub struct Config {
	pub default_database: Option<PathBuf>,
	pub keyfile: Option<PathBuf>,
	pub kdf_iterations: u64,
	pub auto_exit: Duration,
	pub theme: Option<String>,
	pub tmux_copy: bool,
	pub tmux_clear: Duration,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			default_database: None,
			keyfile: None,
			kdf_iterations: KDF_ITERATIONS,
			auto_exit: Duration::from_secs(AUTO_EXIT),
			theme: None,
			tmux_copy: false,
			tmux_clear: Duration::from_secs(TMUX_CLEAR),
		}
	}
}

#[derive(Deserialize, Default)]
struct ConfigFile {
	database: Option<String>,
	default_database: Option<String>,
	keyfile: Option<String>,
	kdf_iterations: Option<u64>,
	auto_exit: Option<u64>,
	theme: Option<String>,
	tmux_copy: Option<bool>,
	tmux_clear: Option<u64>,
}

pub fn load_config() -> anyhow::Result<Config> {
	let path = expand_tilde(CONFIGFILE);

	if !path.exists() {
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
		}

		let mut options = fs::OpenOptions::new();
		options.write(true).create_new(true);
		let result = options.open(&path);
		match result {
			Ok(mut file) => {
				use std::io::Write;
				file.write_all(SAMPLE_CONFIG.as_bytes()).with_context(|| format!("couldn't write {}", path.display()))?;
			}
			Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
			Err(error) => {
				return Err(error).with_context(|| format!("couldn't create {}", path.display()));
			}
		}
	}

	let text = fs::read_to_string(&path).with_context(|| format!("couldn't read {}", path.display()))?;
	parse_config(&text)
}

fn parse_config(text: &str) -> anyhow::Result<Config> {
	let raw: ConfigFile = toml::from_str(text)?;
	let defaults = Config::default();
	Ok(Config {
		default_database: raw.database.or(raw.default_database).map(|s| expand_tilde(&s)),
		keyfile: raw.keyfile.filter(|s| !s.is_empty()).map(|s| expand_tilde(&s)),
		kdf_iterations: raw.kdf_iterations.unwrap_or(defaults.kdf_iterations),
		auto_exit: raw.auto_exit.map(Duration::from_secs).unwrap_or(defaults.auto_exit),
		theme: raw.theme,
		tmux_copy: raw.tmux_copy.unwrap_or(defaults.tmux_copy),
		tmux_clear: raw.tmux_clear.filter(|&secs| secs > 0).map(Duration::from_secs).unwrap_or(defaults.tmux_clear),
	})
}

#[derive(Serialize)]
struct ConfigFileOut<'a> {
	database: Option<String>,
	keyfile: String,
	kdf_iterations: u64,
	auto_exit: u64,
	theme: Option<&'a str>,
	tmux_copy: bool,
	tmux_clear: u64,
}

pub fn save_config(config: &Config) -> anyhow::Result<()> {
	let path = expand_tilde(CONFIGFILE);

	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
	}

	let text = serialize_config(config)?;
	let tmp_path = sibling_tmp_path(&path);

	let mut options = fs::OpenOptions::new();
	options.write(true).create_new(true);

	let mut file = options.open(&tmp_path).with_context(|| format!("couldn't create {}", tmp_path.display()))?;

	let result = (|| {
		use std::io::Write;

		file.write_all(text.as_bytes()).with_context(|| format!("couldn't write {}", tmp_path.display()))?;

		file.sync_all().with_context(|| format!("couldn't sync {}", tmp_path.display()))?;

		fs::rename(&tmp_path, &path).with_context(|| format!("couldn't replace {}", path.display()))?;

		Ok(())
	})();

	if result.is_err() {
		let _ = fs::remove_file(&tmp_path);
	}

	result
}

fn serialize_config(config: &Config) -> anyhow::Result<String> {
	let out = ConfigFileOut {
		database: config.default_database.as_ref().map(|p| p.display().to_string()),
		keyfile: config.keyfile.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
		kdf_iterations: config.kdf_iterations,
		auto_exit: config.auto_exit.as_secs(),
		theme: config.theme.as_deref(),
		tmux_copy: config.tmux_copy,
		tmux_clear: config.tmux_clear.as_secs(),
	};

	toml::to_string_pretty(&out).context("couldn't serialize config")
}

#[cfg(test)]
mod tests {
	use super::{Config, parse_config, serialize_config};
	use std::path::PathBuf;
	use std::time::Duration;

	#[test]
	fn tmux_copy_is_off_by_default() {
		let config = parse_config("database = \"/tmp/passwords.kdbx\"").expect("config should parse");

		assert!(!config.tmux_copy);
		assert_eq!(config.tmux_clear, Duration::from_secs(30));
	}

	#[test]
	fn tmux_copy_settings_round_trip() {
		let config = parse_config(
			r#"
            tmux_copy = true
            tmux_clear = 5
        "#,
		)
		.expect("config should parse");

		assert!(config.tmux_copy);
		assert_eq!(config.tmux_clear, Duration::from_secs(5));

		let text = serialize_config(&config).expect("config should serialize");
		assert!(text.contains("tmux_copy = true"));
		assert!(text.contains("tmux_clear = 5"));
	}

	#[test]
	fn tmux_clear_of_zero_falls_back_to_the_default() {
		let config = parse_config("tmux_clear = 0").expect("config should parse");

		assert_eq!(config.tmux_clear, Duration::from_secs(30));
	}

	#[test]
	fn parses_optional_keyfile_path() {
		let config = parse_config(
			r#"
                database = "/tmp/passwords.kdbx"
                keyfile = "/tmp/passwords.keyx"
            "#,
		)
		.expect("config should parse");

		assert_eq!(config.default_database, Some(PathBuf::from("/tmp/passwords.kdbx")));
		assert_eq!(config.keyfile, Some(PathBuf::from("/tmp/passwords.keyx")));
	}

	#[test]
	fn keyfile_remains_optional() {
		let config = parse_config("database = \"/tmp/passwords.kdbx\"").expect("config should parse");

		assert_eq!(config.keyfile, None);

		let text = serialize_config(&config).expect("config should serialize");
		assert!(text.contains("keyfile = \"\""));
	}

	#[test]
	fn empty_keyfile_means_no_keyfile() {
		let config = parse_config(
			r#"
            database = "/tmp/passwords.kdbx"
            keyfile = ""
        "#,
		)
		.expect("config should parse");

		assert_eq!(config.keyfile, None);
	}

	#[test]
	fn serializes_keyfile_path() {
		let config = Config { keyfile: Some(PathBuf::from("/tmp/passwords.keyx")), ..Config::default() };

		let text = serialize_config(&config).expect("config should serialize");
		assert!(text.contains("keyfile = \"/tmp/passwords.keyx\""));
		assert!(!text.contains("database = "));
	}

	#[test]
	fn parses_legacy_default_database_key() {
		let config = parse_config(r#"default_database = "/tmp/passwords.kdbx""#).expect("legacy config should parse");

		assert_eq!(config.default_database, Some(PathBuf::from("/tmp/passwords.kdbx")));
	}

	#[test]
	fn database_key_takes_precedence_over_legacy_key() {
		let config = parse_config(
			r#"
            database = "/tmp/new.kdbx"
            default_database = "/tmp/old.kdbx"
        "#,
		)
		.expect("config should parse");

		assert_eq!(config.default_database, Some(PathBuf::from("/tmp/new.kdbx")));
	}
}
