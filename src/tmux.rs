//! Opt-in tmux copying. Named buffers have a server timer and are deleted on paste.
//! Deleting a tmux buffer does not guarantee zeroization of tmux's memory.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail, ensure};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
pub const MAX_CLEAR_SECONDS: u64 = 86_400;
const OPTION: &str = "@kptui-buffer";

pub fn available() -> bool {
	std::env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

#[derive(Clone)]
struct Server {
	socket: PathBuf,
	session: String,
}

impl Server {
	fn current() -> anyhow::Result<Self> {
		// TMUX contains socket path, server PID, session ID. Split from the
		// right because the socket path itself may contain commas.
		Self::parse(&std::env::var("TMUX").context("not running inside tmux")?)
	}

	fn parse(value: &str) -> anyhow::Result<Self> {
		let mut parts = value.rsplitn(3, ',');
		let session = parts.next().unwrap_or_default();
		let pid = parts.next().unwrap_or_default();
		let socket = parts.next().unwrap_or_default();
		ensure!(!socket.is_empty() && digits(pid) && digits(session), "invalid TMUX environment");
		Ok(Self { socket: socket.into(), session: format!("${session}") })
	}

	fn run(&self, args: &[&str], input: Option<&[u8]>, capture: bool) -> anyhow::Result<Vec<u8>> {
		let deadline = Instant::now() + COMMAND_TIMEOUT;
		let mut command = Command::new("tmux");
		command.arg("-S").arg(&self.socket).args(args).stderr(Stdio::null());
		command.stdout(if capture { Stdio::piped() } else { Stdio::null() });
		let mut writer = if input.is_some() {
			let (reader, writer) = UnixStream::pair()?;
			writer.set_nonblocking(true)?;
			command.stdin(Stdio::from(std::os::fd::OwnedFd::from(reader)));
			Some(writer)
		} else {
			command.stdin(Stdio::null());
			None
		};
		let child = command.spawn().context("couldn't run tmux")?;
		drop(command);
		let mut child = PendingChild(child);
		if let (Some(writer), Some(mut bytes)) = (writer.as_mut(), input) {
			while !bytes.is_empty() {
				ensure!(Instant::now() < deadline, "tmux timed out while receiving the value");
				match writer.write(bytes) {
					Ok(0) => bail!("tmux stopped receiving the value"),
					Ok(count) => bytes = &bytes[count..],
					Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
					Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
					Err(err) => return Err(err).context("couldn't send the value to tmux"),
				}
			}
		}
		drop(writer); // EOF completes load-buffer, including after a partial transfer.
		loop {
			if let Some(status) = child.0.try_wait()? {
				ensure!(status.success(), "tmux command failed");
				let mut output = Vec::new();
				if let Some(stdout) = child.0.stdout.take() {
					stdout.take(65_537).read_to_end(&mut output)?;
					ensure!(output.len() <= 65_536, "tmux metadata response is too large");
				}
				return Ok(output);
			}
			ensure!(Instant::now() < deadline, "tmux command timed out");
			std::thread::sleep(Duration::from_millis(5));
		}
	}

	fn delete(&self, name: &str) -> anyhow::Result<()> {
		if self.run(&["delete-buffer", "-b", name], None, false).is_ok() {
			return Ok(());
		}
		// Expiry or paste-and-delete may have removed it already. This
		// checks only metadata; never read a secret back from tmux.
		let names = self.run(&["list-buffers", "-F", "#{buffer_name}"], None, true)?;
		ensure!(!String::from_utf8_lossy(&names).lines().any(|found| found == name), "couldn't delete tmux buffer {name}");
		Ok(())
	}
}

struct PendingChild(Child);
impl Drop for PendingChild {
	fn drop(&mut self) {
		if !matches!(self.0.try_wait(), Ok(Some(_))) {
			let _ = self.0.kill();
			let _ = self.0.wait();
		}
	}
}

struct OwnedBuffer {
	server: Server,
	name: String,
}

#[derive(Default)]
pub struct TmuxBuffer {
	buffer: Option<OwnedBuffer>,
}

impl TmuxBuffer {
	pub fn copy(&mut self, value: &str, clear_after: Duration) -> anyhow::Result<()> {
		ensure!((1..=MAX_CLEAR_SECONDS).contains(&clear_after.as_secs()), "Clear-tmux delay must be 1–86400 seconds");
		self.clear()?;
		let server = Server::current()?;
		let mut random = [0u8; 16];
		File::open("/dev/urandom").context("couldn't generate a unique buffer name")?.read_exact(&mut random)?;
		let name = format!("kptui-{:032x}", u128::from_ne_bytes(random));
		self.buffer = Some(OwnedBuffer { server: server.clone(), name: name.clone() });
		let delay = clear_after.as_secs().to_string();
		let deletion = format!("delete-buffer -b {name}");
		// One parsed command list: unsupported timer syntax fails before
		// loading. No secret is included in this command list or in argv.
		let result = server.run(
			&[
				"load-buffer",
				"-b",
				&name,
				"-",
				";",
				"run-shell",
				"-b",
				"-d",
				&delay,
				"-C",
				&deletion,
				";",
				"set-option",
				"-t",
				&server.session,
				OPTION,
				&name,
			],
			Some(value.as_bytes()),
			false,
		);
		if let Err(error) = result {
			return match self.clear() {
				Ok(()) => Err(error),
				Err(cleanup) => Err(error.context(format!("cleanup also failed: {cleanup:#}; buffer {name} may remain"))),
			};
		}
		Ok(())
	}

	pub fn clear(&mut self) -> anyhow::Result<()> {
		if let Some(buffer) = &self.buffer {
			buffer.server.delete(&buffer.name)?;
		}
		self.buffer = None;
		Ok(())
	}
}

impl Drop for TmuxBuffer {
	fn drop(&mut self) {
		let _ = self.clear();
	}
}

fn digits(value: &str) -> bool {
	!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
	use super::*;

	// The external smoke harness supplies an isolated server and fake values.
	#[test]
	#[ignore = "requires an isolated test server supplied by the smoke harness"]
	fn isolated_entry_shortcut() {
		if std::env::var("KPTUI_TMUX_SMOKE").as_deref() != Ok("entry") {
			return;
		}
		use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
		let server = Server::current().unwrap();
		let mut app = crate::app::start(false, zeroize::Zeroizing::new(String::new()));
		app.screen = crate::app::Screen::Edit;
		app.config.clear_tmux = Duration::from_secs(10);
		let mut entry = crate::db::Entry::default();
		entry.password = "KPTUI_TEST_HIDDEN_PASSWORD".to_string();
		app.edit_entry = Some(entry);
		app.edit_state.select(Some(2));
		assert!(!app.show_password);
		crate::app::handle_key(&mut app, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
		assert!(app.status.as_deref().unwrap().contains("copied to tmux"));
		let name = app.tmux_buffer.buffer.as_ref().unwrap().name.clone();
		assert_eq!(server.run(&["show-buffer", "-b", &name], None, true).unwrap(), b"KPTUI_TEST_HIDDEN_PASSWORD");
		app.screen = crate::app::Screen::Settings;
		app.settings_state.select(Some(crate::input::settings::TMUX_CLEAR_ROW));
		app.field_editor = Some(crate::input::fieldedit::FieldEditor::with_text("0"));
		app.editing_field = true;
		crate::app::handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
		assert!(app.config.clear_tmux.is_zero());
		assert!(app.tmux_buffer.buffer.is_none());
		assert!(server.run(&["show-buffer", "-b", &name], None, true).is_err());
		app.screen = crate::app::Screen::Edit;
		crate::app::handle_key(&mut app, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
		assert!(app.tmux_buffer.buffer.is_none());
	}

	#[test]
	#[ignore = "requires an isolated test server supplied by the smoke harness"]
	fn isolated_server_helper() {
		let Ok(mode) = std::env::var("KPTUI_TMUX_SMOKE") else { return };
		let ttl: u64 = std::env::var("KPTUI_TMUX_TTL").unwrap().parse().unwrap();
		let mut buffer = TmuxBuffer::default();
		let copied = buffer.copy("KPTUI_TEST_SENTINEL", Duration::from_secs(ttl));
		if mode == "failure" {
			assert!(copied.is_err());
			println!("FAILURE_HANDLED");
			return;
		}
		copied.unwrap();
		let first = buffer.buffer.as_ref().unwrap().name.clone();
		if mode == "replace" {
			buffer.copy("KPTUI_TEST_REPLACEMENT", Duration::from_secs(ttl + 3)).unwrap();
		}
		println!("READY {first} {}", buffer.buffer.as_ref().unwrap().name);
		std::io::stdout().flush().unwrap();
		if mode != "drop" {
			std::thread::sleep(Duration::from_secs(30));
		}
	}

	#[test]
	fn parses_tmux_endpoint_without_using_its_session_as_a_command() {
		let server = Server::parse("/tmp/socket,with,commas,1234,7").unwrap();
		assert_eq!(server.socket, PathBuf::from("/tmp/socket,with,commas"));
		assert_eq!(server.session, "$7");
		for value in ["", "socket", "socket,1,", "socket,1,7;kill-server", ",1,7"] {
			assert!(Server::parse(value).is_err());
		}
	}
}
