use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::util::expand_tilde;

const SAMPLE_CONFIG: &str = include_str!("../config.toml");
const AUTO_EXIT: u64 = 0;
const CONFIGFILE: &str = "~/.config/kptui/config.toml";
static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn is_config_filename(name: &str) -> bool {
	name.len() > 5 && !name.contains(['/', '\\']) && Path::new(name).extension().is_some_and(|ext| ext == "toml")
}

pub fn select_config(name: &str) -> anyhow::Result<()> {
	anyhow::ensure!(is_config_filename(name), "expected a filename ending in .toml");
	let path = expand_tilde("~/.config/kptui").join(name);
	CONFIG_PATH.set(path).map_err(|_| anyhow::anyhow!("configuration file already selected"))
}

pub fn config_name() -> String {
	config_path().file_stem().expect("config filename has a stem").to_string_lossy().into_owned()
}

fn config_path() -> PathBuf {
	CONFIG_PATH.get().cloned().unwrap_or_else(|| expand_tilde(CONFIGFILE))
}

const KDF_ITERATIONS: u64 = 90;
pub const MIB: u64 = 1024 * 1024;
pub const DEFAULT_KDF_MEMORY: u64 = 256 * MIB;

fn sibling_tmp_path(path: &Path) -> PathBuf {
	let mut tmp = path.as_os_str().to_os_string();
	tmp.push(".tmp");
	PathBuf::from(tmp)
}

pub struct Config {
	pub default_database: Option<PathBuf>,
	pub keyfile: Option<PathBuf>,
	pub kdf_iterations: u64,
	pub kdf_memory: u64,
	pub auto_exit: Duration,
	pub theme: Option<String>,
	/// When tmux deletes a copied value (0: copying to tmux is off).
	pub tmux_clear: Duration,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			default_database: None,
			keyfile: None,
			kdf_iterations: KDF_ITERATIONS,
			kdf_memory: DEFAULT_KDF_MEMORY,
			auto_exit: Duration::from_secs(AUTO_EXIT),
			theme: None,
			tmux_clear: Duration::ZERO,
		}
	}
}

#[derive(Deserialize, Default)]
struct ConfigFile {
	database: Option<String>,
	default_database: Option<String>,
	keyfile: Option<String>,
	kdf_iterations: Option<u64>,
	kdf_memory: Option<KdfMemoryValue>,
	auto_exit: Option<u64>,
	theme: Option<String>,
	tmux_clear: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum KdfMemoryValue {
	Bytes(u64),
	WithUnit(String),
}

impl KdfMemoryValue {
	fn into_bytes(self) -> anyhow::Result<u64> {
		match self {
			Self::Bytes(bytes) => Ok(bytes),
			Self::WithUnit(text) => {
				let value = text.trim().to_ascii_lowercase();
				let number = value
					.strip_suffix("mib")
					.or_else(|| value.strip_suffix("mb"))
					.or_else(|| value.strip_suffix('m'))
					.context(r#"kdf_memory requires a byte count or a quoted whole-MiB value such as "256M""#)?;
				let mib = number.trim().parse::<u64>().context("kdf_memory must contain a whole number of MiB")?;
				anyhow::ensure!(mib > 0, "kdf_memory must be at least 1 MiB");
				mib.checked_mul(MIB).context("kdf_memory is too large")
			}
		}
	}
}

pub fn load_config() -> anyhow::Result<Config> {
	let path = config_path();
	let initial = if CONFIG_PATH.get().is_some() { serialize_config(&Config::default())? } else { SAMPLE_CONFIG.to_string() };
	load_config_at(&path, &initial)
}

fn load_config_at(path: &Path, initial: &str) -> anyhow::Result<Config> {
	if !path.exists() {
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent).with_context(|| format!("couldn't create directory {}", parent.display()))?;
		}

		let mut options = fs::OpenOptions::new();
		options.write(true).create_new(true);
		let result = options.open(path);
		match result {
			Ok(mut file) => {
				use std::io::Write;
				file.write_all(initial.as_bytes()).with_context(|| format!("couldn't write file {}", path.display()))?;
			}
			Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
			Err(error) => {
				return Err(error).with_context(|| format!("couldn't create file {}", path.display()));
			}
		}
	}

	let text = fs::read_to_string(path).with_context(|| format!("couldn't read file {}", path.display()))?;
	parse_config(&text)
}

fn parse_config(text: &str) -> anyhow::Result<Config> {
	let raw: ConfigFile = toml::from_str(text)?;
	let defaults = Config::default();
	Ok(Config {
		default_database: raw.database.or(raw.default_database).map(|s| expand_tilde(&s)),
		keyfile: raw.keyfile.filter(|s| !s.is_empty()).map(|s| expand_tilde(&s)),
		kdf_iterations: raw.kdf_iterations.unwrap_or(defaults.kdf_iterations),
		kdf_memory: raw.kdf_memory.map(KdfMemoryValue::into_bytes).transpose()?.unwrap_or(defaults.kdf_memory),
		auto_exit: raw.auto_exit.map(Duration::from_secs).unwrap_or(defaults.auto_exit),
		theme: raw.theme,
		tmux_clear: raw.tmux_clear.map(Duration::from_secs).unwrap_or(defaults.tmux_clear),
	})
}

#[derive(Serialize)]
struct ConfigFileOut<'a> {
	database: Option<String>,
	keyfile: String,
	kdf_iterations: u64,
	kdf_memory: KdfMemoryValue,
	auto_exit: u64,
	theme: Option<&'a str>,
	tmux_clear: u64,
}

pub fn save_config(config: &Config) -> anyhow::Result<()> {
	save_config_at(config, &config_path())
}

fn save_config_at(config: &Config, path: &Path) -> anyhow::Result<()> {
	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
	}

	let text = serialize_config(config)?;
	let tmp_path = sibling_tmp_path(path);

	let mut options = fs::OpenOptions::new();
	options.write(true).create_new(true);

	let mut file = options.open(&tmp_path).with_context(|| format!("couldn't create {}", tmp_path.display()))?;

	let result = (|| {
		use std::io::Write;

		file.write_all(text.as_bytes()).with_context(|| format!("couldn't write {}", tmp_path.display()))?;

		file.sync_all().with_context(|| format!("couldn't sync {}", tmp_path.display()))?;

		fs::rename(&tmp_path, path).with_context(|| format!("couldn't replace {}", path.display()))?;

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
		kdf_memory: if config.kdf_memory > 0 && config.kdf_memory.is_multiple_of(MIB) {
			KdfMemoryValue::WithUnit(format!("{}M", config.kdf_memory / MIB))
		} else {
			KdfMemoryValue::Bytes(config.kdf_memory)
		},
		auto_exit: config.auto_exit.as_secs(),
		theme: config.theme.as_deref(),
		tmux_clear: config.tmux_clear.as_secs(),
	};

	toml::to_string_pretty(&out).context("couldn't serialize config")
}

#[cfg(test)]
mod tests {
	use super::{Config, SAMPLE_CONFIG, parse_config, serialize_config};
	use std::path::PathBuf;
	use std::time::Duration;

	#[test]
	fn copying_to_tmux_is_off_by_default() {
		for text in ["", "tmux_clear = 0", SAMPLE_CONFIG] {
			assert_eq!(parse_config(text).expect("config should parse").tmux_clear, Duration::ZERO, "{text}");
		}
	}

	#[test]
	fn tmux_clear_round_trips() {
		let config = parse_config("tmux_clear = 5").expect("config should parse");
		assert_eq!(config.tmux_clear, Duration::from_secs(5));
		assert!(serialize_config(&config).expect("config should serialize").contains("tmux_clear = 5"));
	}

	#[test]
	fn selected_file_is_created_with_defaults_and_saved_independently() {
		let directory = std::env::temp_dir().join(format!("kptui-config-argument-{}", std::process::id()));
		let selected = directory.join("nested/Work.toml");
		let default_path = directory.join("nested/config.toml");
		let initial = serialize_config(&Config::default()).unwrap();
		let mut config = super::load_config_at(&selected, &initial).unwrap();
		assert!(selected.is_file());
		assert!(config.default_database.is_none());
		assert!(config.keyfile.is_none());
		assert_eq!(config.kdf_iterations, Config::default().kdf_iterations);
		assert_eq!(config.kdf_memory, Config::default().kdf_memory);
		assert_eq!(config.auto_exit, Config::default().auto_exit);
		assert!(config.theme.is_none());
		std::fs::write(&default_path, "unchanged").unwrap();
		config.kdf_iterations = 42;
		super::save_config_at(&config, &selected).unwrap();
		let loaded = super::load_config_at(&selected, &initial).unwrap();
		assert_eq!(loaded.kdf_iterations, 42);
		assert_eq!(std::fs::read_to_string(default_path).unwrap(), "unchanged");
		std::fs::remove_dir_all(directory).unwrap();
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
	#[test]
	fn memory_defaults_and_legacy_values_round_trip() {
		assert_eq!(parse_config("").unwrap().kdf_memory, 256 * super::MIB);
		for bytes in [262_144, 268_435_456] {
			let text = format!("kdf_memory = {bytes}");
			let config = parse_config(&text).unwrap();
			assert_eq!(config.kdf_memory, bytes);
			let saved = serialize_config(&config).unwrap();
			assert_eq!(parse_config(&saved).unwrap().kdf_memory, bytes);
		}
	}
	#[test]
	fn memory_accepts_mib_suffixes_and_saves_the_short_form() {
		for text in ["256m", "256M", "256MB", "256MiB", "256mb", "256mib"] {
			let config = parse_config(&format!("kdf_memory = {text:?}")).unwrap();
			assert_eq!(config.kdf_memory, 256 * super::MIB);
			let saved = serialize_config(&config).unwrap();
			assert!(saved.contains(r#"kdf_memory = "256M""#));
			assert_eq!(parse_config(&saved).unwrap().kdf_memory, config.kdf_memory);
		}
		let config = parse_config("kdf_memory = 268435456").unwrap();
		assert!(serialize_config(&config).unwrap().contains(r#"kdf_memory = "256M""#));
		let legacy = parse_config("kdf_memory = 262144").unwrap();
		assert!(serialize_config(&legacy).unwrap().contains("kdf_memory = 262144"));
	}

	#[test]
	fn memory_rejects_invalid_units_fractions_and_overflow() {
		for text in ["0M", "-1M", "1.5M", "256GB", "256", "18446744073709551615M", ""] {
			assert!(parse_config(&format!("kdf_memory = {text:?}")).is_err(), "{text}");
		}
	}
}
