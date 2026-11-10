use ratatui::{
	Frame,
	layout::{Constraint, Direction, Layout},
	style::Style,
	text::{Line, Span},
	widgets::{Cell, Padding, Paragraph, Row, Table},
};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::db::Entry;
use crate::theme::Theme;
use crate::util::wrap_help_items;

pub(crate) const SEARCH_HEIGHT: u16 = 2;

pub(crate) fn header_height(slim_mode: bool) -> u16 {
	u16::from(!slim_mode)
}

fn help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode {
		&["[^s]", "[Enter]", "[Esc]", "[^a]", "[^d]", "[^c]"]
	} else {
		&["[^s] Search", "[Enter] View", "[Esc] Quit", "[^a] Add", "[^d] Delete", "[^c] Config"]
	}
}

fn masked_user<'a>(entry: &'a Entry, theme: &Theme) -> Line<'a> {
	let normal = Style::new().fg(theme.text);
	let spans = vec![Span::styled(entry.user.as_str(), normal)];
	Line::from(spans)
}

fn slim_row<'a>(entry: &'a Entry, theme: &Theme) -> Line<'a> {
	let normal = Style::new().fg(theme.text);
	let spans = vec![Span::styled(entry.name.as_str(), normal)];
	Line::from(spans)
}

pub fn draw_index(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();
	let help_width = full_area.width;
	let help_lines = wrap_help_items(help_items(app.slim_mode), help_width);
	let help_height = help_lines.len() as u16;
	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Length(SEARCH_HEIGHT), Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);
	let help_text = help_lines.join("\n");
	let help = if let Some(status) = &app.status {
		// Replace the help text with the latest status message.
		Paragraph::new(status.as_str()).style(Style::new().fg(app.theme.warning))
	} else {
		Paragraph::new(help_text).style(Style::new().fg(app.theme.accent))
	};
	frame.render_widget(help, vertical[2]);

	let rows = app.filtered.iter().map(|&i| {
		let entry = &app.entries[i];
		let user = masked_user(entry, &app.theme);
		if app.slim_mode {
			Row::new([Cell::from(slim_row(entry, &app.theme))])
		} else {
			Row::new([Cell::from(entry.name.as_str()), Cell::from(user), Cell::from(entry.date_last_modify.as_str())])
		}
	});

	let column_widths = if app.slim_mode {
		vec![Constraint::Percentage(100)]
	} else {
		vec![Constraint::Percentage(30), Constraint::Percentage(40), Constraint::Fill(1)]
	};
	let mut table_index = Table::new(rows, column_widths)
		.column_spacing(1)
		.style(Style::new().fg(app.theme.text))
		.row_highlight_style(Style::new().fg(app.theme.selection_fg).bg(app.theme.selection_bg).bold());
	if !app.slim_mode {
		table_index = table_index.header(Row::new(["Name", "User", "Modified"]).style(Style::new().bold().fg(app.theme.header)));
	}
	let index_area = vertical[1];
	let header_height = header_height(app.slim_mode);
	app.index_page_size = index_area.height.saturating_sub(header_height).max(1) as usize;
	frame.render_stateful_widget(table_index, index_area, &mut app.index_state);

	let query_area = vertical[0];
	let title = if app.search_notes { "Index [Name/User/Notes] " } else { "Index [Name/User] " };
	let section = super::section(title, &app.theme, 0).padding(Padding::ZERO);
	let field = section.inner(query_area);
	app.max_len = usize::from(field.width).min(crate::input::SECRET_CAP);
	let input_text = app.query.clone();
	let input = Paragraph::new(input_text.as_str());
	let cursor_column = UnicodeWidthStr::width(input_text.as_str());
	let cursor_x = field.x.saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX).min(field.width.saturating_sub(1)));
	frame.set_cursor_position((cursor_x, field.y));
	super::draw_input(frame, query_area, section, input, &app.theme);
}
