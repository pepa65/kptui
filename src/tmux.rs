//! Optional copying of entry fields into a tmux paste buffer, for terminals
//! without a system clipboard (like the Linux console).
//!
//! - The value is piped into `tmux load-buffer -`, so it never appears in argv
//!   or in the environment.
//! - It becomes the newest automatic buffer, so the usual `prefix ]` pastes it.
//! - tmux doesn't tell which buffer that is, so it's found by its contents: only
//!   buffers holding exactly the copied value are ever deleted.
//! - Its deletion after the configured delay is scheduled in the tmux server, so
//!   it also happens when kptui gets killed or its pane is closed. kptui deletes
//!   it on exit, too.

use std::io::{ErrorKind, Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use zeroize::Zeroizing;

const ATTEMPTS: usize = 3;

/// Whether kptui runs inside a tmux session.
pub fn available() -> bool {
	std::env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Buffer {
	name: String,
	size: usize,
}

/// The tmux buffers holding the last copied value, deleted on drop. It's only
/// more than one when something else copied the same value at the same moment.
#[derive(Default)]
pub struct TmuxBuffer {
	names: Vec<String>,
}

impl TmuxBuffer {
	/// Copies `value` into a new tmux buffer, replacing the previously copied
	/// one, and has the tmux server delete it after `clear_after` (at least 1s).
	/// The error says whether the value was copied.
	pub fn copy(&mut self, value: &str, clear_after: Duration) -> anyhow::Result<()> {
		self.clear();
		let value = value.as_bytes();
		let before = buffers().context("not copied")?;
		load_buffer(value).context("not copied")?;
		self.names = find_copies(&before, value).context("copied, but can't clear it")?;
		ensure!(!self.names.is_empty(), "not copied: tmux dropped the buffer");

		let delay = clear_after.as_secs().max(1).to_string();
		for name in &self.names {
			if run(&["run-shell", "-b", "-d", &delay, "-C", &format!("delete-buffer -b {name}")]).is_err() {
				// Without the deletion in the server, a killed kptui would leave the value behind.
				self.clear();
				bail!("not copied: needs tmux 3.2+");
			}
		}
		Ok(())
	}

	/// Deletes the copied value from tmux, if it is still there.
	pub fn clear(&mut self) {
		for name in self.names.drain(..) {
			let _ = run(&["delete-buffer", "-b", &name]);
		}
	}
}

impl Drop for TmuxBuffer {
	fn drop(&mut self) {
		self.clear();
	}
}

/// The buffers that hold `value` and weren't there `before`.
fn find_copies(before: &[Buffer], value: &[u8]) -> anyhow::Result<Vec<String>> {
	let mut attempt = 1;
	loop {
		match buffers().and_then(|after| new_copies(before, &after, value.len(), |name| holds(name, value))) {
			Ok(names) => return Ok(names),
			Err(err) if attempt == ATTEMPTS => return Err(err),
			Err(_) => {
				attempt += 1;
				std::thread::sleep(Duration::from_millis(100));
			}
		}
	}
}

/// Picks the copies of a value of `size` bytes among the new buffers.
fn new_copies(before: &[Buffer], after: &[Buffer], size: usize, holds: impl Fn(&str) -> anyhow::Result<bool>) -> anyhow::Result<Vec<String>> {
	let mut names = Vec::new();
	for buffer in after {
		// The name gets embedded in a tmux command, and automatic names are plain.
		let candidate = buffer.size == size && is_plain_name(&buffer.name) && !before.iter().any(|old| old.name == buffer.name);
		if candidate && holds(&buffer.name)? {
			names.push(buffer.name.clone());
		}
	}
	Ok(names)
}

/// Automatic tmux buffer names look like `buffer12`.
fn is_plain_name(name: &str) -> bool {
	!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn parse_buffers(text: &str) -> Vec<Buffer> {
	text.lines()
		.filter_map(|line| {
			let (size, name) = line.split_once(' ')?;
			Some(Buffer { name: name.to_string(), size: size.parse().ok()? })
		})
		.collect()
}

fn tmux(args: &[&str]) -> Command {
	let mut command = Command::new("tmux");
	command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
	command
}

fn run(args: &[&str]) -> anyhow::Result<()> {
	let status = tmux(args).status().context("couldn't run tmux")?;
	ensure!(status.success(), "tmux {} failed", args[0]);
	Ok(())
}

fn buffers() -> anyhow::Result<Vec<Buffer>> {
	let output = tmux(&["list-buffers", "-F", "#{buffer_size} #{buffer_name}"])
		.stdout(Stdio::piped())
		.output()
		.context("couldn't run tmux")?;
	ensure!(output.status.success(), "tmux list-buffers failed");
	Ok(parse_buffers(&String::from_utf8_lossy(&output.stdout)))
}

fn load_buffer(bytes: &[u8]) -> anyhow::Result<()> {
	let mut child = tmux(&["load-buffer", "-"]).stdin(Stdio::piped()).spawn().context("couldn't run tmux")?;
	// Dropping stdin after writing closes the pipe, so tmux sees end of input.
	let written = child.stdin.take().map(|mut stdin| stdin.write_all(bytes));
	let status = child.wait().context("couldn't run tmux")?;
	ensure!(status.success() && matches!(written, Some(Ok(()))), "tmux load-buffer failed");
	Ok(())
}

/// Whether a buffer holds exactly `value`.
fn holds(name: &str, value: &[u8]) -> anyhow::Result<bool> {
	let mut child = tmux(&["show-buffer", "-b", name]).stdout(Stdio::piped()).spawn().context("couldn't run tmux")?;
	// Fixed size, as a growing Vec would leave unwiped copies behind. One byte
	// more than needed, to notice a longer buffer.
	let mut bytes = Zeroizing::new(vec![0u8; value.len() + 1]);
	let mut filled = 0;
	if let Some(mut stdout) = child.stdout.take() {
		while filled < bytes.len() {
			match stdout.read(&mut bytes[filled..]) {
				Ok(0) => break,
				Ok(count) => filled += count,
				Err(err) if err.kind() == ErrorKind::Interrupted => {}
				Err(_) => break,
			}
		}
	}
	let status = child.wait().context("couldn't run tmux")?;
	if filled > value.len() {
		return Ok(false);
	}
	ensure!(status.success(), "tmux show-buffer failed");
	Ok(bytes[..filled] == *value)
}

#[cfg(test)]
mod tests {
	use super::{Buffer, is_plain_name, new_copies, parse_buffers};

	fn buffer(name: &str, size: usize) -> Buffer {
		Buffer { name: name.to_string(), size }
	}

	#[test]
	fn only_plain_buffer_names_are_accepted() {
		assert!(is_plain_name("buffer0"));
		assert!(is_plain_name("buffer123"));
		assert!(!is_plain_name(""));
		assert!(!is_plain_name("buffer0; kill-server"));
		assert!(!is_plain_name("buffer 1"));
		assert!(!is_plain_name("'buffer1'"));
	}

	#[test]
	fn parses_the_list_of_buffers() {
		assert_eq!(parse_buffers("12 buffer1\n3 my notes\nnonsense\n"), [buffer("buffer1", 12), buffer("my notes", 3)]);
		assert!(parse_buffers("").is_empty());
	}

	#[test]
	fn finds_its_buffer_among_others_that_appear_at_the_same_moment() {
		let before = [buffer("buffer0", 6)];
		let after = [buffer("buffer3", 6), buffer("buffer2", 6), buffer("buffer1", 4), buffer("buffer0", 6)];
		let copies = new_copies(&before, &after, 6, |name| {
			assert!(name == "buffer2" || name == "buffer3", "only new buffers of the same size are read, not {name}");
			Ok(name == "buffer2")
		});
		assert_eq!(copies.expect("buffers should be readable"), ["buffer2"]);
	}

	#[test]
	fn a_buffer_that_cannot_be_read_is_an_error_instead_of_a_guess() {
		assert!(new_copies(&[], &[buffer("buffer0", 6)], 6, |_| anyhow::bail!("tmux failed")).is_err());
		assert!(new_copies(&[], &[], 6, |_| Ok(true)).expect("nothing to read").is_empty());
		assert!(new_copies(&[], &[buffer("not plain", 6)], 6, |_| Ok(true)).expect("nothing to read").is_empty());
	}
}
