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

pub(crate) struct Recovery {
	original: zeroize::Zeroizing<String>,
	table: toml::Table,
	invalid: Vec<String>,
}

pub struct Config {
	pub default_database: Option<PathBuf>,
	pub keyfile: Option<PathBuf>,
	pub kdf_iterations: u64,
	pub kdf_memory: u64,
	pub auto_exit: Duration,
	pub theme: Option<String>,
	pub(crate) recovery: Option<Recovery>,
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
			recovery: None,
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
	let initial = if CONFIG_PATH.get().is_some() {
		serialize_config(&Config { default_database: Some(crate::util::default_new_database_path()), ..Config::default() })?
	} else {
		SAMPLE_CONFIG.to_string()
	};
	load_config_at(&path, &initial)
}

fn load_config_at(path: &Path, initial: &str) -> anyhow::Result<Config> {
	let text = match fs::read_to_string(path) {
		Ok(text) => text,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return parse_startup_config(initial),
		Err(error) => return Err(error).with_context(|| format!("couldn't read file {}", path.display())),
	};
	parse_startup_config(&text).with_context(|| format!("couldn't parse file {}", path.display()))
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
		recovery: None,
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
}

pub fn save_config(config: &Config) -> anyhow::Result<()> {
	anyhow::ensure!(!config.is_recovering(), "configuration recovery: saves are blocked until all invalid fields are corrected");
	save_config_at(config, &config_path())
}

fn save_config_at(config: &Config, path: &Path) -> anyhow::Result<()> {
	write_config_at(&serialize_config(config)?, path)
}

fn write_config_at(text: &str, path: &Path) -> anyhow::Result<()> {
	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
	}

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
	};

	toml::to_string_pretty(&out).context("couldn't serialize config")
}

const CONFIG_KEYS: &[&str] = &["database", "default_database", "keyfile", "kdf_iterations", "kdf_memory", "auto_exit", "theme"];

impl Config {
	pub fn is_recovering(&self) -> bool {
		self.recovery.is_some()
	}

	pub fn field_invalid(&self, key: &str) -> bool {
		self.recovery
			.as_ref()
			.is_some_and(|recovery| recovery.invalid.iter().any(|bad| bad == key || (key == "database" && bad == "default_database")))
	}

	pub fn recovery_message(&self) -> Option<String> {
		self.recovery.as_ref().map(|recovery| {
			if recovery.invalid.is_empty() {
				"Configuration corrected but not saved. Reconfirm a setting to retry, or Esc to quit.".to_string()
			} else {
				format!("Invalid: {}. Saves blocked until corrected; Esc quits.", recovery.invalid.join(", "))
			}
		})
	}
}

// TOML syntax must be valid before offering recovery. Validate each field
// independently, keeping every usable setting instead of discarding the file.
fn parse_startup_config(text: &str) -> anyhow::Result<Config> {
	let table: toml::Table = toml::from_str(text)?;
	if let Ok(config) = parse_config(text) {
		return Ok(config);
	}
	let mut usable = table.clone();
	let mut invalid = Vec::new();
	for &key in CONFIG_KEYS {
		if let Some(value) = table.get(key) {
			let mut single = toml::Table::new();
			single.insert(key.to_string(), value.clone());
			if parse_config(&toml::to_string(&single)?).is_err() {
				usable.remove(key);
				invalid.push(key.to_string());
			}
		}
	}
	let mut config = parse_config(&toml::to_string(&usable)?)?;
	config.recovery = Some(Recovery { original: zeroize::Zeroizing::new(text.to_string()), table, invalid });
	Ok(config)
}

// A successful edit repairs just that field in the original document. Only
// when the entire candidate validates may it be saved. Unknown keys survive.
pub fn repair_config_field(config: &mut Config, key: &str) -> anyhow::Result<Option<PathBuf>> {
	let corrected: toml::Table = toml::from_str(&serialize_config(config)?)?;
	let recovery = config.recovery.as_mut().context("not recovering configuration")?;
	recovery.table.remove(key);
	if key == "database" {
		recovery.table.remove("default_database");
	}
	if let Some(value) = corrected.get(key) {
		recovery.table.insert(key.to_string(), value.clone());
	}
	recovery.invalid.retain(|bad| bad != key && !(key == "database" && bad == "default_database"));
	let text = toml::to_string_pretty(&recovery.table)?;
	if parse_config(&text).is_err() {
		return Ok(None);
	}
	let path = config_path();
	anyhow::ensure!(
		fs::read_to_string(&path)? == *recovery.original,
		"configuration changed externally; quit and reopen to avoid overwriting it"
	);
	let mut backup = path.as_os_str().to_os_string();
	backup.push(format!(
		".before-repair-{}-{}",
		std::process::id(),
		std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()
	));
	let backup = PathBuf::from(backup);
	let mut options = fs::OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}
	{
		use std::io::Write;
		let mut file = options.open(&backup).context("couldn't back up the original configuration")?;
		file.write_all(recovery.original.as_bytes())?;
		file.sync_all()?;
	}
	write_config_at(&text, &path)?;
	config.recovery = None;
	Ok(Some(backup))
}

#[cfg(test)]
mod tests {
	use super::{Config, parse_config, serialize_config};
	use std::path::PathBuf;

	#[test]
	fn recovery_blocks_saves_until_all_bad_fields_are_fixed() {
		let path = super::config_path();
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		let original = "database = \"/keep/vault.kdbx\"\nkeyfile = \"/keep/vault.keyx\"\nauto_exit = -1\nkdf_memory = \"bad\"\nfuture_setting = \"keep me\"\n";
		std::fs::write(&path, original).unwrap();
		let mut config = super::load_config().unwrap();
		assert!(config.field_invalid("auto_exit") && config.field_invalid("kdf_memory"));
		assert!(super::save_config(&config).is_err());
		config.auto_exit = std::time::Duration::from_secs(17);
		assert!(super::repair_config_field(&mut config, "auto_exit").unwrap().is_none());
		assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
		assert!(super::save_config(&config).is_err());
		config.kdf_memory = 2 * super::MIB;
		let backup = super::repair_config_field(&mut config, "kdf_memory").unwrap().unwrap();
		assert!(!config.is_recovering());
		assert_eq!(std::fs::read_to_string(backup).unwrap(), original);
		let saved = std::fs::read_to_string(path).unwrap();
		let table: toml::Table = toml::from_str(&saved).unwrap();
		assert_eq!(table["future_setting"].as_str(), Some("keep me"));
		let saved = parse_config(&saved).unwrap();
		assert_eq!(saved.default_database, Some(PathBuf::from("/keep/vault.kdbx")));
		assert_eq!(saved.keyfile, Some(PathBuf::from("/keep/vault.keyx")));
		assert_eq!(saved.auto_exit.as_secs(), 17);
		assert_eq!(saved.kdf_memory, 2 * super::MIB);
	}

	#[test]
	fn recovery_never_overwrites_an_external_edit() {
		let path = super::config_path();
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(&path, "auto_exit = -1\n").unwrap();
		let mut config = super::load_config().unwrap();
		let external = "auto_exit = 23\n";
		std::fs::write(&path, external).unwrap();
		config.auto_exit = std::time::Duration::from_secs(5);
		assert!(super::repair_config_field(&mut config, "auto_exit").is_err());
		assert_eq!(std::fs::read_to_string(path).unwrap(), external);
	}

	#[test]
	fn selected_file_stays_in_memory_until_saved() {
		let directory = std::env::temp_dir().join(format!("kptui-config-argument-{}", std::process::id()));
		let selected = directory.join("nested/Work.toml");
		let default_path = directory.join("nested/config.toml");
		let initial = serialize_config(&Config::default()).unwrap();
		let mut config = super::load_config_at(&selected, &initial).unwrap();
		assert!(!selected.exists());
		assert!(!selected.parent().unwrap().exists());
		assert!(config.default_database.is_none());
		assert!(config.keyfile.is_none());
		assert_eq!(config.kdf_iterations, Config::default().kdf_iterations);
		assert_eq!(config.kdf_memory, Config::default().kdf_memory);
		assert_eq!(config.auto_exit, Config::default().auto_exit);
		assert!(config.theme.is_none());
		std::fs::create_dir_all(default_path.parent().unwrap()).unwrap();
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
