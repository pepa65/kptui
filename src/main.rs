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
use zeroize::Zeroizing;

use crate::util::secret_pop;

fn main() {
	let slim_mode = handle_cli_flags();
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
	let mut terminal = ratatui::init();
	let result = app::run(&mut terminal, slim_mode, password);
	ratatui::restore();
	let mut out = stdout();
	out.write_all(b"\x1b[H\x1b[2J\x1b[3J").unwrap();
	out.flush().unwrap();
	if let Err(err) = result {
		eprintln!("kptui: {err}");
		std::process::exit(1);
	}
}

fn handle_cli_flags() -> bool {
	let args: Vec<String> = std::env::args().skip(1).collect();

	if args.iter().any(|a| a == "--version" || a == "-V" || a == "version") {
		println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
		std::process::exit(0);
	}

	if args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
		println!("{} {} - {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_DESCRIPTION"));
		println!("Usage:  {} [--slim | --version | --help]", env!("CARGO_PKG_NAME"));
		std::process::exit(0);
	}

	args.iter().any(|a| a == "--slim" || a == "-s" || a == "slim")
}
