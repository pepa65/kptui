use std::io::{self, IsTerminal, Write};

use crossterm::{
	cursor::{MoveTo, Show},
	event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
	execute,
	style::ResetColor,
	terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
	Terminal,
	backend::CrosstermBackend,
	widgets::{Paragraph, Wrap},
};

const README: &str = include_str!("../README.md");
const IGNORE_TOP_LINES: usize = 7;

pub fn run() -> io::Result<()> {
	if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
		return io::stdout().lock().write_all(README.as_bytes());
	}
	let mut guard = ReadmeTerminal::new()?;
	guard.terminal.clear()?;
	let mut top = Paragraph::new(README.lines().take(IGNORE_TOP_LINES).collect::<Vec<_>>().join("\n"))
		.wrap(Wrap { trim: false })
		.line_count(guard.terminal.size()?.width);
	loop {
		let size = guard.terminal.size()?;
		let height = usize::from(size.height).max(1);
		let paragraph = Paragraph::new(README).wrap(Wrap { trim: false });
		let lines = paragraph.line_count(size.width);
		let max_top = lines.saturating_sub(height);
		top = top.min(max_top);
		guard.terminal.draw(|frame| {
			frame.render_widget(paragraph.scroll((top as u16, 0)), frame.area());
		})?;
		let Event::Key(key) = event::read()? else { continue };
		if key.kind == KeyEventKind::Release {
			continue;
		}
		match key.code {
			KeyCode::Char('q') | KeyCode::Esc => break,
			KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
			KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter => top = top.saturating_add(1),
			KeyCode::Up | KeyCode::Char('k') => top = top.saturating_sub(1),
			KeyCode::PageDown | KeyCode::Char(' ') => top = top.saturating_add(height),
			KeyCode::PageUp | KeyCode::Char('b') => top = top.saturating_sub(height),
			KeyCode::Home | KeyCode::Char('g') => {
				top = 0;
			}
			KeyCode::End | KeyCode::Char('G') => top = max_top,
			_ => {}
		}
	}
	Ok(())
}

// README pages are public: keep the last page on the main screen when closing.
struct ReadmeTerminal {
	terminal: ratatui::DefaultTerminal,
}

impl ReadmeTerminal {
	fn new() -> io::Result<Self> {
		let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
		enable_raw_mode()?;
		Ok(Self { terminal })
	}
}

impl Drop for ReadmeTerminal {
	fn drop(&mut self) {
		let _ = disable_raw_mode();
		let mut out = io::stdout();
		let bottom = self.terminal.size().map(|size| size.height.saturating_sub(1)).unwrap_or(0);
		let _ = execute!(out, ResetColor, Show, MoveTo(0, bottom));
		let _ = out.write_all(b"\r\n");
		let _ = out.flush();
	}
}
