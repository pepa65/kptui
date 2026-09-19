mod app;
mod config;
mod db;
mod export;
mod import;
mod input;
mod theme;
mod ui;
mod util;

use std::io;
use std::io::{stdout, Write};


fn main() -> io::Result<()> {
	let slim_mode = handle_cli_flags();
	let mut terminal = ratatui::init();
	app::run(&mut terminal, slim_mode)?;
	ratatui::restore();
	let mut out = stdout();
	out.write_all(b"\x1b[H\x1b[2J\x1b[3J").unwrap();
	out.flush().unwrap();
	Ok(())
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
