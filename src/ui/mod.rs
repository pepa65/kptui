use ratatui::{
	Frame,
	layout::Rect,
	style::Style,
	widgets::{Block, Padding, Paragraph},
};

use crate::theme::Theme;

pub mod edit;
pub mod index;
pub mod login;
pub mod settings;

fn mask_password(text: &str) -> String {
	"•".repeat(text.chars().count())
}

fn section<'a>(title: &'a str, theme: &Theme, inset: u16) -> Block<'a> {
	// Keep the content's insets and reserve the title row with no border glyphs.
	Block::new().title(title).title_style(Style::new().fg(theme.error)).padding(Padding::new(inset, inset, 0, 1))
}

fn input_style(theme: &Theme) -> Style {
	Style::new().fg(theme.text).bg(theme.border)
}

fn draw_input(frame: &mut Frame, area: Rect, section: Block<'_>, input: Paragraph<'_>, theme: &Theme) {
	frame.buffer_mut().set_style(section.inner(area), input_style(theme));
	frame.render_widget(input.block(section), area);
}

#[cfg(test)]
mod tests {
	use super::*;
	use ratatui::{Terminal, backend::TestBackend};

	#[test]
	fn input_background_covers_the_field_and_leaves_the_title_plain() {
		let theme = crate::theme::Theme::default();
		let area = Rect::new(0, 0, 20, 4);
		let section = section(" Entry ", &theme, 2);
		assert_eq!(section.inner(area), Rect::new(2, 1, 16, 2));

		let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
		let frame = terminal.draw(|frame| draw_input(frame, area, section, Paragraph::new("Name"), &theme)).unwrap();
		let buffer = frame.buffer;
		assert_eq!(buffer[(1, 0)].symbol(), "E");
		assert_eq!(buffer[(1, 0)].fg, theme.error);
		assert_eq!(buffer[(1, 0)].bg, ratatui::style::Color::Reset);
		assert_eq!(buffer[(2, 1)].symbol(), "N");
		assert_eq!(buffer[(2, 1)].bg, theme.border);
		assert_eq!(buffer[(17, 1)].symbol(), " ");
		assert_eq!(buffer[(17, 1)].bg, theme.border);
		for y in 0..area.height {
			assert_eq!(buffer[(area.width - 1, y)].symbol(), " ");
		}
		assert_eq!(buffer[(0, 1)].symbol(), " ");
		assert_eq!(buffer[(1, area.height - 1)].symbol(), " ");
	}
}
