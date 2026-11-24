use std::path::PathBuf;

const VAULT: &str = "~/.local/share/kptui/default.kdbx";

fn strip_surrounding_quotes(path: &str) -> &str {
	let bytes = path.as_bytes();

	if bytes.len() >= 2 {
		let first = bytes[0];
		let last = bytes[bytes.len() - 1];

		if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
			return &path[1..path.len() - 1];
		}
	}

	path
}

pub fn expand_tilde(path: &str) -> PathBuf {
	let path = strip_surrounding_quotes(path.trim());

	if let Some(rest) = path.strip_prefix("~/")
		&& let Some(home) = home_dir()
	{
		return home.join(rest);
	}
	PathBuf::from(path)
}

pub fn default_new_database_path() -> PathBuf {
	expand_tilde(VAULT)
}

pub fn wrap_help_items(items: &[&str], width: u16) -> Vec<String> {
	let width = width as usize;
	let mut lines: Vec<String> = Vec::new();
	let mut current = String::new();

	for item in items {
		let extra = if current.is_empty() { 0 } else { 2 };

		if !current.is_empty() && current.len() + extra + item.len() > width {
			lines.push(std::mem::take(&mut current));
		}

		if !current.is_empty() {
			current.push_str("  ");
		}
		current.push_str(item);
	}

	if !current.is_empty() || lines.is_empty() {
		lines.push(current);
	}

	lines
}

#[cfg(not(test))]
fn home_dir() -> Option<PathBuf> {
	std::env::var_os("HOME").map(PathBuf::from)
}

// Tests must never read or write the user's configuration, themes, or vaults.
// Each test thread gets its own home without changing process-wide environment variables.
#[cfg(test)]
fn home_dir() -> Option<PathBuf> {
	thread_local! {
		static HOME: TestHome = TestHome::new();
	}
	Some(HOME.with(|home| home.0.clone()))
}

#[cfg(test)]
struct TestHome(PathBuf);

#[cfg(test)]
impl TestHome {
	fn new() -> Self {
		use std::sync::atomic::{AtomicU64, Ordering};
		static NEXT_HOME: AtomicU64 = AtomicU64::new(0);
		loop {
			let id = NEXT_HOME.fetch_add(1, Ordering::Relaxed);
			let path = std::env::temp_dir().join(format!("kptui-test-home-{}-{id}", std::process::id()));
			match std::fs::create_dir(&path) {
				Ok(()) => return Self(path),
				Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
				Err(err) => panic!("couldn't create test home: {err}"),
			}
		}
	}
}

#[cfg(test)]
impl Drop for TestHome {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

#[cfg(test)]
mod home_tests {
	use super::*;

	#[test]
	fn test_config_writes_are_isolated_between_threads() {
		let path = expand_tilde("~/.config/kptui/config.toml");
		assert!(path.starts_with(std::env::temp_dir()));
		assert_eq!(expand_tilde("~/.config/kptui/config.toml"), path);
		let mut config = crate::config::Config::default();
		config.kdf_iterations = 7;
		crate::config::save_config(&config).unwrap();
		assert_eq!(crate::config::load_config().unwrap().kdf_iterations, 7);
		let other = std::thread::spawn(|| {
			let path = expand_tilde("~/.config/kptui/config.toml");
			assert!(!path.exists());
			let mut config = crate::config::Config::default();
			config.kdf_iterations = 11;
			crate::config::save_config(&config).unwrap();
			assert_eq!(crate::config::load_config().unwrap().kdf_iterations, 11);
			path
		})
		.join()
		.unwrap();
		assert_ne!(path, other);
		assert!(!other.exists());
		assert_eq!(crate::config::load_config().unwrap().kdf_iterations, 7);
	}
}
