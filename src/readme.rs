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
	let mut pager = Pager::new(guard.terminal.size()?.width);
	loop {
		guard.terminal.draw(|frame| pager.draw(frame))?;
		if let Event::Key(key) = event::read()?
			&& pager.key(key)
		{
			break;
		}
	}
	Ok(())
}

pub(crate) struct Pager {
	top: usize,
	height: usize,
	max_top: usize,
}

impl Pager {
	pub(crate) fn new(width: u16) -> Self {
		let top = Paragraph::new(README.lines().take(IGNORE_TOP_LINES).collect::<Vec<_>>().join("\n"))
			.wrap(Wrap { trim: false })
			.line_count(width);
		Self { top, height: 1, max_top: 0 }
	}

	pub(crate) fn draw(&mut self, frame: &mut ratatui::Frame) {
		let area = frame.area();
		let paragraph = Paragraph::new(README).wrap(Wrap { trim: false });
		self.height = usize::from(area.height).max(1);
		self.max_top = paragraph.line_count(area.width).saturating_sub(self.height);
		self.top = self.top.min(self.max_top);
		frame.render_widget(paragraph.scroll((self.top as u16, 0)), area);
	}

	// Returns true when the viewer should close.
	pub(crate) fn key(&mut self, key: crossterm::event::KeyEvent) -> bool {
		if key.kind == KeyEventKind::Release {
			return false;
		}
		match key.code {
			KeyCode::Char('q') | KeyCode::Esc => return true,
			KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
			KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter => self.top = self.top.saturating_add(1),
			KeyCode::Up | KeyCode::Char('k') => self.top = self.top.saturating_sub(1),
			KeyCode::PageDown | KeyCode::Char(' ') => self.top = self.top.saturating_add(self.height),
			KeyCode::PageUp | KeyCode::Char('b') => self.top = self.top.saturating_sub(self.height),
			KeyCode::Home | KeyCode::Char('g') => self.top = 0,
			KeyCode::End | KeyCode::Char('G') => self.top = self.max_top,
			_ => {}
		}
		false
	}
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
