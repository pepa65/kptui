//! Optional copying of entry fields into a tmux paste buffer, for terminals
//! without a system clipboard (like the Linux console).
//!
//! - The value is piped into `tmux load-buffer -`, so it never appears in argv
//!   or in the environment, and only buffer names are ever read back from tmux.
//! - It becomes the newest automatic buffer, so the usual `prefix ]` pastes it.
//! - Its deletion after the configured delay is always scheduled in the tmux
//!   server, so it also happens when kptui gets killed or its pane is closed.
//!   kptui deletes it on exit, too.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail};

/// Whether kptui runs inside a tmux session.
pub fn available() -> bool {
	std::env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

/// What happened to a copied value.
pub enum Copied {
	/// The tmux server deletes it after the delay.
	Clearing,
	/// Another buffer appeared at the same moment, so kptui can't tell which
	/// one is its own and leaves both alone.
	Untracked,
}

/// The tmux buffer holding the last copied value, deleted on drop.
#[derive(Default)]
pub struct TmuxBuffer {
	name: Option<String>,
}

impl TmuxBuffer {
	/// Copies `value` into a new tmux buffer, replacing the previously copied
	/// one, and has the tmux server delete it after `clear_after` (at least 1s).
	pub fn copy(&mut self, value: &str, clear_after: Duration) -> anyhow::Result<Copied> {
		self.clear();
		let before = buffer_names()?;
		load_buffer(value.as_bytes())?;
		let mut added = buffer_names().unwrap_or_default().into_iter().filter(|name| !before.contains(name));
		let (Some(name), None) = (added.next(), added.next()) else {
			return Ok(Copied::Untracked);
		};
		if !is_plain_name(&name) {
			return Ok(Copied::Untracked);
		}

		let delay = clear_after.as_secs().max(1).to_string();
		let scheduled = tmux(&["run-shell", "-b", "-d", &delay, "-C", &format!("delete-buffer -b {name}")]);
		self.name = Some(name);
		if scheduled.is_err() {
			// Without the deletion in the server, a killed kptui would leave the value behind.
			self.clear();
			bail!("needs tmux 3.2+");
		}
		Ok(Copied::Clearing)
	}

	/// Deletes the copied value from tmux, if it is still there.
	pub fn clear(&mut self) {
		if let Some(name) = self.name.take() {
			let _ = tmux(&["delete-buffer", "-b", &name]);
		}
	}
}

impl Drop for TmuxBuffer {
	fn drop(&mut self) {
		self.clear();
	}
}

/// Automatic tmux buffer names look like `buffer12`; anything else is refused
/// before it gets embedded in a tmux command.
fn is_plain_name(name: &str) -> bool {
	!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn tmux(args: &[&str]) -> anyhow::Result<()> {
	let status = Command::new("tmux")
		.args(args)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status()
		.context("couldn't run tmux")?;
	if !status.success() {
		bail!("tmux {} failed", args[0]);
	}
	Ok(())
}

fn buffer_names() -> anyhow::Result<Vec<String>> {
	let output = Command::new("tmux")
		.args(["list-buffers", "-F", "#{buffer_name}"])
		.stdin(Stdio::null())
		.stderr(Stdio::null())
		.output()
		.context("couldn't run tmux")?;
	if !output.status.success() {
		bail!("tmux list-buffers failed");
	}
	Ok(String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned).collect())
}

fn load_buffer(bytes: &[u8]) -> anyhow::Result<()> {
	let mut child = Command::new("tmux")
		.args(["load-buffer", "-"])
		.stdin(Stdio::piped())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.context("couldn't run tmux")?;
	// Dropping stdin after writing closes the pipe, so tmux sees end of input.
	let written = child.stdin.take().map(|mut stdin| stdin.write_all(bytes));
	let status = child.wait().context("couldn't run tmux")?;
	if !status.success() || !matches!(written, Some(Ok(()))) {
		bail!("tmux load-buffer failed");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::is_plain_name;

	#[test]
	fn only_plain_buffer_names_are_accepted() {
		assert!(is_plain_name("buffer0"));
		assert!(is_plain_name("buffer123"));
		assert!(!is_plain_name(""));
		assert!(!is_plain_name("buffer0; kill-server"));
		assert!(!is_plain_name("buffer 1"));
		assert!(!is_plain_name("'buffer1'"));
	}
}
