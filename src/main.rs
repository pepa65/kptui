mod app;
mod config;
mod db;
mod export;
mod import;
mod input;
mod theme;
mod ui;
mod util;

use std::io::{self, IsTerminal, Write, stdout};
use std::process::exit;
use zeroize::Zeroizing;

use crate::input::secret_pop;

enum Command {
	Help,
	Version,
	Tui(bool),
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

fn read_stdin() -> Zeroizing<String> {
	let mut password = Zeroizing::new(String::new());
	if !io::stdin().is_terminal() {
		io::stdin().read_line(&mut password).unwrap();
		if password.ends_with('\n') {
			secret_pop(&mut password);
			if password.ends_with('\r') {
				secret_pop(&mut password);
			}
		}
	}
	password
}

fn reset_terminal() {
	let mut out = stdout();
	out.write_all(b"\x1b[H\x1b[2J\x1b[3J").unwrap();
	out.flush().unwrap();
}

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let command = match parse_args(&args) {
		Ok(command) => command,
		Err(error) => {
			eprintln!("kptui: {error}");
			eprintln!("Usage: {} [slim | version | help]", env!("CARGO_PKG_NAME"));
			exit(2);
		}
	};

	match command {
		Command::Help => {
			println!("{} {} - {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_DESCRIPTION"));
			println!("Usage:  {} [slim | version | help]", env!("CARGO_PKG_NAME"));
			exit(0);
		}

		Command::Version => {
			println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
			exit(0);
		}

		Command::Tui(slim) => {
			let password = read_stdin();
			let mut terminal = ratatui::init();
			let result = app::run(&mut terminal, slim, password);
			ratatui::restore();
			reset_terminal();
			if let Err(err) = result {
				eprintln!("kptui: {err}");
				std::process::exit(1);
			}
		}
	}
}
