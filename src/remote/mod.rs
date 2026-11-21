//! Databases on an SFTP server, used through a local copy (like KeePass2Android).
//!
//! kptui opens and saves the local copy, and this module keeps it in sync:
//! - When the database gets selected, the server copy replaces the local one,
//!   unless that has changes that weren't uploaded yet. If the server can't be
//!   reached, the local copy gets used and changes are uploaded later.
//! - After every change, the local copy gets uploaded, if the server copy is
//!   still the one it started from. If not, both get merged first (see `merge`).
//! - Next to the local copy, the last version known to be on the server is kept
//!   (the "base"). Comparing it with the local copy tells whether there are
//!   changes to upload, and with a fresh download whether the server copy
//!   changed. It's also what both sides started from, for merging.
//! - Just opening a database never writes to the server, only changes do.

pub mod merge;
pub mod sftp;

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, anyhow, bail};
use keepass::Database;

use crate::app::{App, Screen};
use crate::util::expand_tilde;
use sftp::{Failure, Target};

const DIR: &str = "~/.local/share/kptui/remote";
const UPLOADING: &str = ", uploading...";

/// A database configured as an `sftp://` URL.
pub struct Remote {
	url: String,
	target: Result<Target, String>,
}

impl Remote {
	pub fn new(url: &str) -> Self {
		let url = url.trim().to_string();
		let target = Target::parse(&url);
		Self { url, target }
	}

	pub fn url(&self) -> &str {
		&self.url
	}

	fn target(&self) -> Result<&Target, Failure> {
		self.target.as_ref().map_err(|err| Failure::Failed(format!("invalid sftp URL: {err}")))
	}

	/// The local copy, which kptui opens and saves.
	pub fn cache_path(&self) -> PathBuf {
		let label = match &self.target {
			Ok(target) => format!("{}-{}", target.host(), target.file_name().trim_end_matches(".kdbx")),
			Err(_) => "invalid".to_string(),
		};
		let label: String = label
			.chars()
			.map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
			.take(64)
			.collect();
		expand_tilde(DIR).join(format!("{label}-{:016x}.kdbx", fnv1a(self.url.as_bytes())))
	}

	/// The last version known to be on the server.
	fn base_path(&self) -> PathBuf {
		with_suffix(&self.cache_path(), ".base")
	}
}

/// How the local copy relates to the server.
#[derive(Default)]
pub struct SyncState {
	/// Shown on the login screen when the server copy couldn't be used.
	pub note: Option<String>,
	/// There's no local copy to fall back on, so there's nothing to open or create.
	pub blocked: bool,
	/// The local copy has changes that aren't on the server yet.
	pub unsynced: bool,
	/// An upload is due, after the next redraw.
	pub pending: bool,
	/// The local copy as of the last check, to notice writes to it.
	seen: Option<(Option<SystemTime>, u64)>,
}

/// Updates the local copy from the server, when the database gets selected
/// (on start, or in the Config screen). This needs no master password.
pub fn open(app: &mut App) {
	app.remote = SyncState::default();
	let Some(remote) = app.config.remote.as_ref() else {
		return;
	};
	let (cache, base) = (remote.cache_path(), remote.base_path());
	let has_cache = cache.is_file();
	let unsynced = has_cache && !same_contents(&cache, &base);

	let problem = match remote.target().and_then(|target| download(target, &cache)) {
		Ok(download) if crate::db::kdbx_version(&download.path).is_none() => Some("the server file isn't a KeePass database".to_string()),
		Ok(_) if unsynced => None,
		Ok(download) => install(download, &cache, &base).err().map(|err| format!("{err:#}")),
		Err(Failure::Missing) if !has_cache => None,
		Err(failure) => Some(failure.to_string()),
	};

	let state = &mut app.remote;
	state.unsynced = unsynced;
	match problem {
		Some(problem) if has_cache => state.note = Some(format!("Using the local copy: {problem}")),
		Some(problem) => {
			state.note = Some(format!("Can't open it: {problem}"));
			state.blocked = true;
		}
		None if unsynced => state.note = Some("Local changes get uploaded after unlocking".to_string()),
		None => {}
	}
}

/// Schedules an upload when the local copy changed since the last check.
/// Called after anything that may have written to it.
pub fn note_writes(app: &mut App) {
	let Some(remote) = app.config.remote.as_ref() else {
		return;
	};
	if app.kdbx.is_none() || app.remote.pending {
		return;
	}

	let cache = remote.cache_path();
	let stamp = stamp(&cache);
	if stamp == app.remote.seen {
		return;
	}
	app.remote.seen = stamp;
	app.remote.unsynced = !same_contents(&cache, &remote.base_path());
	if app.remote.unsynced {
		app.remote.pending = true;
		app.status = Some(match app.status.take() {
			Some(status) if !status.starts_with("Saving") => format!("{status}{UPLOADING}"),
			_ => "Uploading...".to_string(),
		});
	}
}

/// Does the upload scheduled by `note_writes`.
pub fn process_pending(app: &mut App) {
	if !app.remote.pending {
		return;
	}
	app.remote.pending = false;

	let result = sync(app);
	if let Some(remote) = app.config.remote.as_ref() {
		let cache = remote.cache_path();
		app.remote.seen = stamp(&cache);
		app.remote.unsynced = !same_contents(&cache, &remote.base_path());
	}

	let message = result.unwrap_or_else(|err| format!("not uploaded: {err:#}"));
	app.status = Some(match app.status.take().and_then(|status| status.strip_suffix(UPLOADING).map(str::to_string)) {
		Some(done) => format!("{done}; {message}"),
		None => capitalize(&message),
	});
}

/// Uploads the local copy, merging it with the server copy first if that changed.
fn sync(app: &mut App) -> anyhow::Result<String> {
	let Some(remote) = app.config.remote.as_ref() else {
		return Ok(String::new());
	};
	let (cache, base) = (remote.cache_path(), remote.base_path());
	let target = remote.target().map_err(|failure| anyhow!("{failure}"))?.clone();

	let download = match download(&target, &cache) {
		Ok(download) => download,
		Err(Failure::Unreachable(reason)) => return Ok(format!("not uploaded, server unreachable: {reason}")),
		Err(Failure::Missing) => {
			upload(&target, &cache, &base)?;
			return Ok("uploaded".to_string());
		}
		Err(Failure::Failed(reason)) => bail!(reason),
	};

	if crate::db::kdbx_version(&download.path).is_none() {
		bail!("the server file isn't a KeePass database, so it wasn't replaced");
	}
	if same_contents(&download.path, &base) {
		// Same contents, but the download has the server file's current permissions.
		fs::rename(&download.path, &base).context("couldn't store the server copy")?;
		upload(&target, &cache, &base)?;
		return Ok("uploaded".to_string());
	}
	if same_contents(&download.path, &cache) {
		fs::rename(&download.path, &base).context("couldn't store the server copy")?;
		return Ok("already up to date on the server".to_string());
	}
	merge_server_changes(app, &target, download, &cache, &base)
}

fn merge_server_changes(app: &mut App, target: &Target, download: Download, cache: &Path, base: &Path) -> anyhow::Result<String> {
	let key = app.db_key.clone().context("the database is locked")?;
	let server = crate::db::open_database(&download.path, &key).map_err(|_| anyhow!("the server copy changed and doesn't open with this master key"))?;
	let common = if base.is_file() {
		Some(crate::db::open_database(base, &key).map_err(|_| anyhow!("the last uploaded copy doesn't open with this master key"))?)
	} else {
		None
	};
	let local = app.kdbx.as_ref().context("the database is locked")?;
	let (mut merged, report) = merge::merge(common.as_ref(), local, &server);
	drop((server, common));

	crate::db::save_database(cache, &key, &mut merged, &mut [])?;
	// The local copy now includes the server copy, so that's what it started from.
	fs::rename(&download.path, base).context("couldn't store the server copy")?;
	reload(app, merged);

	let notes = merge_notes(&report);
	Ok(match upload(target, cache, base) {
		Ok(()) => format!("merged with changes from the server and uploaded{notes}"),
		Err(err) => format!("merged with changes from the server, not uploaded: {err:#}{notes}"),
	})
}

/// What the merge resolved, to add to the status.
fn merge_notes(report: &merge::Report) -> String {
	let mut notes = Vec::new();
	match report.conflicts {
		0 => {}
		1 => notes.push("1 conflict, kept as a copy".to_string()),
		n => notes.push(format!("{n} conflicts, kept as copies")),
	}
	match report.kept {
		0 => {}
		1 => notes.push("1 deleted entry kept, it was changed".to_string()),
		n => notes.push(format!("{n} deleted entries kept, they were changed")),
	}
	if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) }
}

/// Shows a merged database, keeping the entry being viewed open.
fn reload(app: &mut App, db: Database) {
	let viewed = app.edit_entry.as_ref().and_then(|entry| entry.id).filter(|_| app.screen == Screen::Edit);
	app.entries = crate::db::entries_of(&db);
	app.kdbx = Some(db);
	app.refresh_filter();
	if app.screen != Screen::Edit {
		return;
	}
	match viewed.and_then(|id| app.filtered.iter().position(|&index| app.entries[index].id == Some(id))) {
		Some(position) => {
			app.index_state.select(Some(position));
			crate::input::command::preview_entry(app);
		}
		None => crate::input::edit::reset_edit_state(app),
	}
}

/// A download of the server file, deleted unless it gets moved.
struct Download {
	path: PathBuf,
}

impl Drop for Download {
	fn drop(&mut self) {
		let _ = fs::remove_file(&self.path);
	}
}

fn download(target: &Target, cache: &Path) -> Result<Download, Failure> {
	let (dir, name) = split(cache).map_err(|err| Failure::Failed(err.to_string()))?;
	create_dir(dir).map_err(|err| Failure::Failed(format!("couldn't create {}: {err}", dir.display())))?;
	let name = format!("{name}.download-{}", std::process::id());
	let download = Download { path: dir.join(&name) };
	target.download(dir, &name)?;
	Ok(download)
}

/// Makes a download the base and the local copy.
fn install(download: Download, cache: &Path, base: &Path) -> anyhow::Result<()> {
	fs::rename(&download.path, base).context("couldn't store the server copy")?;
	write_file(cache, &fs::read(base).context("couldn't read the server copy")?, 0o600)
}

/// Uploads the local copy, which then becomes the base. The server file keeps
/// its permissions, which the base has from the download.
fn upload(target: &Target, cache: &Path, base: &Path) -> anyhow::Result<()> {
	let (dir, name) = split(cache)?;
	let mode = file_mode(base);
	target.upload(dir, name, mode).map_err(|failure| anyhow!("{failure}"))?;
	write_file(base, &fs::read(cache).context("couldn't read the local copy")?, mode.unwrap_or(0o600))
}

fn split(path: &Path) -> anyhow::Result<(&Path, &str)> {
	match (path.parent(), path.file_name().and_then(|name| name.to_str())) {
		(Some(dir), Some(name)) => Ok((dir, name)),
		_ => bail!("invalid local path {}", path.display()),
	}
}

fn same_contents(a: &Path, b: &Path) -> bool {
	matches!((fs::read(a), fs::read(b)), (Ok(a), Ok(b)) if a == b)
}

fn stamp(path: &Path) -> Option<(Option<SystemTime>, u64)> {
	fs::metadata(path).ok().map(|metadata| (metadata.modified().ok(), metadata.len()))
}

fn file_mode(path: &Path) -> Option<u32> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		fs::metadata(path).ok().map(|metadata| metadata.permissions().mode() & 0o777)
	}
	#[cfg(not(unix))]
	{
		let _ = path;
		None
	}
}

fn create_dir(dir: &Path) -> std::io::Result<()> {
	let mut builder = fs::DirBuilder::new();
	builder.recursive(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::DirBuilderExt;
		builder.mode(0o700);
	}
	builder.create(dir)
}

/// Replaces a file in one go, with the given permissions.
fn write_file(path: &Path, bytes: &[u8], mode: u32) -> anyhow::Result<()> {
	let tmp = with_suffix(path, ".tmp");
	let _ = fs::remove_file(&tmp);
	let result = (|| -> std::io::Result<()> {
		let mut options = OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
			options.mode(0o600);
			let file = options.open(&tmp)?;
			// Unlike the mode at creation, this isn't reduced by the umask.
			file.set_permissions(fs::Permissions::from_mode(mode))?;
			write_and_sync(file, bytes)?;
		}
		#[cfg(not(unix))]
		{
			let _ = mode;
			write_and_sync(options.open(&tmp)?, bytes)?;
		}
		fs::rename(&tmp, path)
	})();
	if result.is_err() {
		let _ = fs::remove_file(&tmp);
	}
	result.with_context(|| format!("couldn't write {}", path.display()))
}

fn write_and_sync(mut file: fs::File, bytes: &[u8]) -> std::io::Result<()> {
	file.write_all(bytes)?;
	file.sync_all()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
	let mut name = OsString::from(path.as_os_str());
	name.push(suffix);
	PathBuf::from(name)
}

fn capitalize(text: &str) -> String {
	let mut chars = text.chars();
	chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// FNV-1a: a stable hash, unlike std's, to tell local copies apart.
fn fnv1a(bytes: &[u8]) -> u64 {
	bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
	use super::{Remote, capitalize, fnv1a, merge, merge_notes};

	#[test]
	fn local_copies_are_named_after_host_and_file() {
		let home = std::env::var("HOME").unwrap_or_default();
		let path = Remote::new("sftp://me@nas:2222/srv/Sync Configs/My Vault.kdbx").cache_path();
		let name = path.file_name().and_then(|name| name.to_str()).expect("name should be UTF-8");
		assert!(path.starts_with(format!("{home}/.local/share/kptui/remote")));
		assert!(name.starts_with("nas-My_Vault-"), "{name}");
		assert!(name.ends_with(".kdbx"), "{name}");
		assert_ne!(path, Remote::new("sftp://me@nas:2222/srv/Other/My Vault.kdbx").cache_path());
		assert_eq!(path, Remote::new("  sftp://me@nas:2222/srv/Sync Configs/My Vault.kdbx ").cache_path());
	}

	#[test]
	fn hashes_are_stable() {
		assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
		assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
	}

	#[test]
	fn describes_what_the_merge_resolved() {
		assert_eq!(merge_notes(&merge::Report::default()), "");
		assert_eq!(merge_notes(&merge::Report { conflicts: 1, kept: 0 }), " (1 conflict, kept as a copy)");
		assert_eq!(
			merge_notes(&merge::Report { conflicts: 2, kept: 1 }),
			" (2 conflicts, kept as copies; 1 deleted entry kept, it was changed)"
		);
	}

	#[test]
	fn capitalizes_messages() {
		assert_eq!(capitalize("uploaded"), "Uploaded");
		assert_eq!(capitalize(""), "");
	}
}
