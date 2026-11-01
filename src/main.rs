mod app;
mod config;
mod db;
mod export;
mod import;
mod input;
mod theme;
mod ui;
mod util;

use std::fs::File;
use std::io::{self, IsTerminal, Read, Write, stdout};
use std::os::fd::AsFd;
use std::process::ExitCode;
use zeroize::Zeroizing;

enum Command {
	Help,
	Version,
	Tui(bool),
}

fn read_stdin() -> io::Result<Zeroizing<String>> {
	if io::stdin().is_terminal() {
		return Ok(Zeroizing::new(String::with_capacity(crate::input::SECRET_CAP)));
	}

	let mut file = File::from(io::stdin().as_fd().try_clone_to_owned()?);
	// Fixed capacity up front: a growing Vec would leave unwiped old allocations behind.
	let mut bytes: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(crate::input::SECRET_CAP));
	let mut byte = Zeroizing::new([0u8; 1]);
	loop {
		match file.read(&mut *byte) {
			Ok(0) => break,
			Ok(_) => {
				if byte[0] == b'\n' {
					break;
				}

				if bytes.len() >= crate::input::SECRET_CAP {
					return Err(io::Error::new(io::ErrorKind::InvalidData, "password too long"));
				}

				bytes.push(byte[0]);
			}
			Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
			Err(err) => return Err(err),
		}
	}

	if bytes.last() == Some(&b'\r') {
		bytes.pop();
	}

	std::str::from_utf8(&bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "password is not valid UTF-8"))?;

	// Moves the buffer into the String without copying; Zeroizing then owns it.
	let string = String::from_utf8(std::mem::take(&mut *bytes)).expect("UTF-8 validated above");
	Ok(Zeroizing::new(string))
}

fn parse_args(args: &[String]) -> Result<Command, String> {
	match args {
		[] => Ok(Command::Tui(false)),
		[arg, ..] if matches!(arg.as_str(), "help" | "-h" | "--help") => Ok(Command::Help),
		[arg, ..] if matches!(arg.as_str(), "version" | "-V" | "--version") => Ok(Command::Version),
		[arg] if matches!(arg.as_str(), "slim" | "-s" | "--slim") => Ok(Command::Tui(true)),
		_ => Err(format!("invalid argument(s): {}", args.join(" "))),
	}
}

fn reset_terminal() -> io::Result<()> {
	let mut out = stdout();
	out.write_all(b"\x1b[H\x1b[2J\x1b[3J")?;
	out.flush()
}

struct TerminalGuard {
	terminal: ratatui::DefaultTerminal,
}

impl TerminalGuard {
	fn new() -> Self {
		Self { terminal: ratatui::init() }
	}
}

impl Drop for TerminalGuard {
	fn drop(&mut self) {
		let _ = self.terminal.clear();
		let _ = ratatui::try_restore();
		let _ = reset_terminal();
	}
}

#[cfg(target_os = "linux")]
fn harden() {
	use rustix::process::{DumpableBehavior, Resource, Rlimit, set_dumpable_behavior, setrlimit};
	let _ = set_dumpable_behavior(DumpableBehavior::NotDumpable);
	let _ = setrlimit(Resource::Core, Rlimit { current: Some(0), maximum: Some(0) });
}

#[cfg(not(target_os = "linux"))]
fn harden() {}

fn main() -> std::process::ExitCode {
	harden();
	let args: Vec<String> = std::env::args().skip(1).collect();
	let command = match parse_args(&args) {
		Ok(command) => command,
		Err(error) => {
			eprintln!("kptui: {error}");
			eprintln!("Usage: {} [slim | version | help]", env!("CARGO_PKG_NAME"));
			return ExitCode::from(2);
		}
	};

	match command {
		Command::Help => {
			println!("{} {} - {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_DESCRIPTION"));
			println!("Usage:  {} [slim | version | help]", env!("CARGO_PKG_NAME"));
			ExitCode::SUCCESS
		}

		Command::Version => {
			println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
			ExitCode::SUCCESS
		}

		Command::Tui(slim) => {
			let password = match read_stdin() {
				Ok(password) => password,
				Err(err) => {
					eprintln!("kptui: couldn't read password from stdin: {err}");
					return ExitCode::FAILURE;
				}
			};

			let mut guard = TerminalGuard::new();
			let result = app::run(&mut guard.terminal, slim, password);
			drop(guard);
			if let Err(err) = result {
				eprintln!("kptui: {err}");
				ExitCode::FAILURE
			} else {
				ExitCode::SUCCESS
			}
		}
	}
}
