use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::Context;
use keepass::db::{EntryId, EntryMut, EntryRef, Times, fields};
use keepass::{Database, DatabaseKey};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

fn resolve_totp(e: &EntryRef) -> String {
	if let Some(otp) = e.get_raw_otp_value()
		&& !otp.trim().is_empty()
	{
		return otp.to_string();
	}

	let Some(seed) = e.get("TOTP Seed") else {
		return String::new();
	};
	let seed = seed.trim();
	if seed.is_empty() {
		return String::new();
	}

	let (period, digits) = e
		.get("TOTP Settings")
		.and_then(|settings| settings.split_once(';'))
		.and_then(|(p, d)| Some((p.trim().parse::<u32>().ok()?, d.trim().parse::<u32>().ok()?)))
		.unwrap_or((30, 6));

	let label = e.get_title().unwrap_or("");
	let user = e.get_username().unwrap_or("");
	let encoded_label = urlencoding_encode(&format!("{label}:{user}"));
	let encoded_issuer = urlencoding_encode(label);

	format!("otpauth://totp/{encoded_label}?secret={seed}&period={period}&digits={digits}&issuer={encoded_issuer}")
}

pub struct TotpCode {
	pub code: Zeroizing<String>,
	pub valid_for: std::time::Duration,
}

pub fn current_totp_code(raw: &str) -> Option<TotpCode> {
	if raw.trim().is_empty() {
		return None;
	}

	let totp: keepass::db::TOTP = raw.parse().ok()?;
	let otp_code = totp.value_now().ok()?;

	Some(TotpCode { code: Zeroizing::new(otp_code.code), valid_for: otp_code.valid_for })
}

fn urlencoding_encode(input: &str) -> String {
	let mut out = String::with_capacity(input.len());
	for b in input.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
			_ => out.push_str(&format!("%{b:02X}")),
		}
	}
	out
}

#[derive(Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct Entry {
	#[zeroize(skip)]
	pub id: Option<EntryId>,
	pub name: String,
	pub user: String,
	pub password: String,
	pub url: String,
	pub totp: String,
	pub notes: String,
	pub date_last_modify: String,
}

fn format_date_time(dt: chrono::NaiveDateTime) -> String {
	dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn unlock_database(path: &Path, password: &Zeroizing<String>, keyfile_path: Option<&Path>) -> anyhow::Result<(Database, DatabaseKey, Vec<Entry>)> {
	let mut file = File::open(path).with_context(|| format!("couldn't open {}", path.display()))?;

	let key = build_database_key(password, keyfile_path)?;
	let db = Database::open(&mut file, key.clone()).map_err(|err| match keyfile_path {
		Some(_) => anyhow::anyhow!("unlock failed with password + keyfile: {err}"),
		None => anyhow::anyhow!("unlock failed with password: {err}"),
	})?;

	let entries = db
		.iter_all_entries()
		.map(|e| {
			let date_last_modify = e.times.last_modification.map(format_date_time).unwrap_or_default();

			Entry {
				id: Some(e.id()),
				name: e.get_title().unwrap_or("(no title)").to_string(),
				user: e.get_username().unwrap_or_default().to_string(),
				password: e.get_password().unwrap_or_default().to_string(),
				url: e.get_url().unwrap_or_default().to_string(),
				totp: resolve_totp(&e),
				notes: e.get(fields::NOTES).unwrap_or_default().to_string(),
				date_last_modify,
			}
		})
		.collect();

	Ok((db, key, entries))
}

pub fn create_database(path: &Path, password: &Zeroizing<String>, keyfile_path: Option<&Path>) -> anyhow::Result<(Database, DatabaseKey, Vec<Entry>)> {
	if path.exists() {
		anyhow::bail!("a file already exists at {}", path.display());
	}

	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
	}

	let db = Database::new();
	let key = build_database_key(password, keyfile_path)?;

	write_to_disk(path, &key, &db)?;

	Ok((db, key, Vec::new()))
}

pub fn build_database_key(password: &Zeroizing<String>, keyfile_path: Option<&Path>) -> anyhow::Result<DatabaseKey> {
	let mut key = DatabaseKey::new().with_password(password);

	if let Some(path) = keyfile_path {
		let mut keyfile = File::open(path).with_context(|| format!("couldn't open keyfile {}", path.display()))?;

		if keyfile.metadata().with_context(|| format!("couldn't inspect keyfile {}", path.display()))?.len() == 0 {
			anyhow::bail!("keyfile {} is empty", path.display());
		}

		key = key.with_keyfile(&mut keyfile).with_context(|| format!("couldn't read keyfile {}", path.display()))?;
	}

	Ok(key)
}

pub fn export_database(path: &Path, password: &Zeroizing<String>, keyfile_path: Option<&Path>, entries: &[Entry]) -> anyhow::Result<()> {
	if path.exists() {
		anyhow::bail!("a file already exists at {}", path.display());
	}

	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
	}

	let mut db = Database::new();
	for entry in entries {
		let mut entry = entry.clone();
		write_entry(&mut db, &mut entry)?;
	}
	let key = build_database_key(password, keyfile_path)?;
	write_to_disk(path, &key, &db)
}

pub fn save_database(path: &Path, key: &DatabaseKey, db: &mut Database, entries: &mut [Entry]) -> anyhow::Result<()> {
	for entry in entries.iter_mut() {
		write_entry(db, entry)?;
	}

	write_to_disk(path, key, db)
}

fn write_to_disk(path: &Path, key: &DatabaseKey, db: &Database) -> anyhow::Result<()> {
	let tmp_path = sibling_tmp_path(path);

	let mut options = OpenOptions::new();
	options.write(true).create_new(true);

	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}

	let mut file = options.open(&tmp_path).with_context(|| format!("couldn't create {}", tmp_path.display()))?;

	let result = (|| {
		db.save(&mut file, key.clone()).map_err(|err| anyhow::anyhow!("failed to write database: {err}"))?;
		file.sync_all().with_context(|| format!("couldn't sync {}", tmp_path.display()))?;
		fs::rename(&tmp_path, path).with_context(|| format!("couldn't replace {}", path.display()))?;
		Ok(())
	})();

	if result.is_err() {
		let _ = fs::remove_file(&tmp_path);
	}

	result
}

fn write_entry(db: &mut Database, entry: &mut Entry) -> anyhow::Result<()> {
	match entry.id {
		Some(id) => {
			let mut e = db.entry_mut(id).context("entry no longer exists in the database")?;
			apply_fields(&mut e, entry);
			entry.date_last_modify = e.times.last_modification.map(format_date_time).unwrap_or_default();
		}
		None => {
			let mut root = db.root_mut();
			let mut e = root.add_entry();
			apply_fields(&mut e, entry);
			entry.id = Some(e.id());
			entry.date_last_modify = e.times.last_modification.map(format_date_time).unwrap_or_default();
		}
	}

	Ok(())
}

pub fn delete_entry(path: &Path, key: &DatabaseKey, db: &mut Database, id: EntryId) -> anyhow::Result<()> {
	db.entry_mut(id).context("entry no longer exists in the database")?.remove();

	write_to_disk(path, key, db)
}

fn apply_fields(e: &mut EntryMut<'_>, entry: &Entry) {
	e.set_unprotected(fields::TITLE, entry.name.clone());
	e.set_protected(fields::USERNAME, entry.user.clone());
	e.set_protected(fields::PASSWORD, entry.password.clone());
	e.set_unprotected(fields::URL, entry.url.clone());
	e.set_protected(fields::OTP, entry.totp.clone());
	e.set_protected(fields::NOTES, entry.notes.clone());
	e.times.last_modification = Some(Times::now());
}

fn sibling_tmp_path(path: &Path) -> PathBuf {
	let mut name: OsString = path.as_os_str().to_owned();
	name.push(".tmp");
	PathBuf::from(name)
}

#[cfg(test)]
mod tests {
	use super::{build_database_key, create_database, save_database, unlock_database};
	use keepass::db::fields;
	use std::fs;
	use std::path::{Path, PathBuf};
	use std::sync::atomic::{AtomicU64, Ordering};
	use zeroize::Zeroizing;

	static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

	const XML_V1_KEYFILE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<KeyFile>
    <Meta>
        <Version>1.00</Version>
    </Meta>
    <Key>
        <Data>NXyYiJMHg3ls+eBmjbAjWec9lcOToJiofbhNiFMTJMw=</Data>
    </Key>
</KeyFile>"#;

	struct TestDir(PathBuf);

	impl TestDir {
		fn new() -> Self {
			let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
			let path = std::env::temp_dir().join(format!("kptui-test-{}-{id}", std::process::id()));
			fs::create_dir_all(&path).expect("test directory should be created");
			Self(path)
		}

		fn join(&self, path: impl AsRef<Path>) -> PathBuf {
			self.0.join(path)
		}
	}

	impl Drop for TestDir {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}

	#[test]
	fn password_only_databases_still_unlock() {
		let dir = TestDir::new();
		let database_path = dir.join("password-only.kdbx");

		create_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).expect("password-only database should be created");
		unlock_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).expect("password-only database should unlock");
	}

	#[test]
	fn unlock_database_reads_entry_notes() {
		let dir = TestDir::new();
		let database_path = dir.join("notes.kdbx");
		let (mut database, key, mut entries) = create_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).expect("database should be created");

		{
			let mut root = database.root_mut();
			let mut entry = root.add_entry();
			entry.set_unprotected(fields::TITLE, "Entry with notes");
			entry.set_protected(fields::NOTES, "first line\nsecond line");
		}

		save_database(&database_path, &key, &mut database, &mut entries).expect("database should be saved");

		let (_, _, entries) = unlock_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).expect("database should unlock");
		let entry = entries.iter().find(|entry| entry.name == "Entry with notes").expect("entry should be loaded");

		assert_eq!(entry.notes, "first line\nsecond line");
	}

	#[test]
	fn unlock_database_defaults_missing_notes_to_empty() {
		let dir = TestDir::new();
		let database_path = dir.join("no-notes.kdbx");
		let (mut database, key, mut entries) = create_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).expect("database should be created");

		{
			let mut root = database.root_mut();
			let mut entry = root.add_entry();
			entry.set_unprotected(fields::TITLE, "Entry without notes");
		}

		save_database(&database_path, &key, &mut database, &mut entries).expect("database should be saved");

		let (_, _, entries) = unlock_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).expect("database should unlock");
		let entry = entries.iter().find(|entry| entry.name == "Entry without notes").expect("entry should be loaded");

		assert!(entry.notes.is_empty());
	}

	#[test]
	fn password_and_keyfile_databases_require_both_factors() {
		let dir = TestDir::new();
		let database_path = dir.join("composite.kdbx");
		let keyfile_path = dir.join("database.keyx");
		fs::write(&keyfile_path, XML_V1_KEYFILE).expect("XML v1 keyfile should be written");

		create_database(&database_path, &Zeroizing::new("correct horse".to_string()), Some(&keyfile_path)).expect("composite-key database should be created");
		unlock_database(&database_path, &Zeroizing::new("correct horse".to_string()), Some(&keyfile_path)).expect("database should unlock with password and keyfile");

		assert!(unlock_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).is_err());
		assert!(unlock_database(&database_path, &Zeroizing::new("wrong password".to_string()), Some(&keyfile_path)).is_err());
	}

	#[test]
	fn password_change_keeps_keyfile_requirement() {
		let dir = TestDir::new();
		let database_path = dir.join("composite.kdbx");
		let keyfile_path = dir.join("database.keyx");
		fs::write(&keyfile_path, [42_u8; 32]).expect("keyfile should be written");

		let (mut database, _, mut entries) =
			create_database(&database_path, &Zeroizing::new("old password".to_string()), Some(&keyfile_path)).expect("composite-key database should be created");
		let new_key = build_database_key(&Zeroizing::new("new password".to_string()), Some(&keyfile_path)).expect("new composite key should be built");

		save_database(&database_path, &new_key, &mut database, &mut entries).expect("database should be re-encrypted");

		unlock_database(&database_path, &Zeroizing::new("new password".to_string()), Some(&keyfile_path)).expect("new password and keyfile should unlock");
		assert!(unlock_database(&database_path, &Zeroizing::new("new password".to_string()), None).is_err());
		assert!(unlock_database(&database_path, &Zeroizing::new("old password".to_string()), Some(&keyfile_path)).is_err());
	}

	#[test]
	fn missing_and_empty_keyfiles_have_clear_errors() {
		let dir = TestDir::new();
		let database_path = dir.join("placeholder.kdbx");
		let missing_keyfile = dir.join("missing.keyx");
		let empty_keyfile = dir.join("empty.keyx");
		fs::write(&database_path, []).expect("placeholder database should be written");
		fs::write(&empty_keyfile, []).expect("empty keyfile should be written");

		let missing_error = unlock_database(&database_path, &Zeroizing::new("secret".to_string()), Some(&missing_keyfile))
			.err()
			.expect("missing keyfile should fail")
			.to_string();
		assert!(missing_error.contains("couldn't open keyfile"));
		assert!(missing_error.contains(&missing_keyfile.display().to_string()));

		let empty_error = unlock_database(&database_path, &Zeroizing::new("secret".to_string()), Some(&empty_keyfile)).err().expect("empty keyfile should fail").to_string();
		assert!(empty_error.contains("keyfile"));
		assert!(empty_error.contains("is empty"));
	}

	#[test]
	fn wrong_keyfile_error_preserves_the_root_cause() {
		let dir = TestDir::new();
		let database_path = dir.join("composite.kdbx");
		let correct_keyfile = dir.join("correct.keyx");
		let wrong_keyfile = dir.join("wrong.keyx");
		fs::write(&correct_keyfile, [7_u8; 32]).expect("correct keyfile should be written");
		fs::write(&wrong_keyfile, [9_u8; 32]).expect("wrong keyfile should be written");

		create_database(&database_path, &Zeroizing::new("secret".to_string()), Some(&correct_keyfile)).expect("composite-key database should be created");

		let error = unlock_database(&database_path, &Zeroizing::new("secret".to_string()), Some(&wrong_keyfile)).err().expect("wrong keyfile should fail").to_string();
		assert!(error.contains("password + keyfile"));
		assert!(error.contains("Incorrect key"));
	}

	#[cfg(unix)]
	#[test]
	fn save_database_rejects_existing_tmp_symlink() {
		use std::os::unix::fs::symlink;

		let dir = TestDir::new();
		let database_path = dir.join("database.kdbx");
		let target_path = dir.join("attacker-target");
		let tmp_path = dir.join("database.kdbx.tmp");

		// Create a valid database first. This initial save needs an
		// unobstructed temporary path.
		let (mut database, key, mut entries) = create_database(&database_path, &Zeroizing::new("correct horse".to_string()), None).expect("database should be created");

		// The initial save should have consumed its temporary file.
		assert!(!tmp_path.exists(), "temporary file should not remain after successful creation");

		// Simulate an attacker planting a symlink at the predictable
		// temporary path.
		fs::write(&target_path, b"must remain untouched").expect("target should be created");
		symlink(&target_path, &tmp_path).expect("tmp symlink should be created");

		let error = save_database(&database_path, &key, &mut database, &mut entries).expect_err("save should reject the existing tmp symlink");

		assert!(error.to_string().contains("couldn't create"), "unexpected error: {error:#}");

		assert_eq!(fs::read(&target_path).expect("target should remain readable"), b"must remain untouched");

		assert!(fs::symlink_metadata(&tmp_path).expect("tmp path should remain").file_type().is_symlink());
	}
}
