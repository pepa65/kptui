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
use crate::groups::GroupInfo;
use crate::theme::Theme;
use crate::util::wrap_help_items;
use keepass::db::GroupId;

pub(crate) const SEARCH_HEIGHT: u16 = 2;

pub(crate) fn header_height(slim_mode: bool) -> u16 {
	u16::from(!slim_mode)
}

fn help_items(slim_mode: bool, esc: &str) -> &'static [&'static str] {
	match (slim_mode, esc) {
		(true, _) => &["[^f]", "[Enter]", "[Esc]", "[^n]", "[^d]", "[^c]"],
		(false, "Clear") => &["[^f] Filter", "[Enter] View", "[Esc] Clear", "[^n] New", "[^d] Delete", "[^c] Config"],
		(false, "Up") => &["[^f] Filter", "[Enter] View", "[Esc] Up", "[^n] New", "[^d] Delete", "[^c] Config"],
		(false, _) => &["[^f] Filter", "[Enter] View", "[Esc] Quit", "[^n] New", "[^d] Delete", "[^c] Config"],
	}
}

/// What Esc does on the Index (see `input::index`).
fn esc_action(app: &App) -> &'static str {
	if !app.groups.is_empty() && !app.query.is_empty() {
		"Clear"
	} else if app.group.is_some() {
		"Up"
	} else {
		"Quit"
	}
}

/// What a group holds, like "2 groups, 5 entries".
fn group_contents(groups: &[GroupInfo], entries: &[Entry], id: GroupId) -> String {
	let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
	let groups = groups.iter().filter(|group| group.parent == Some(id)).count();
	let entries = entries.iter().filter(|entry| entry.group == Some(id)).count();
	match (groups, entries) {
		(0, 0) => "empty".to_string(),
		(0, entries) => count(entries, "entry", "entries"),
		(groups, 0) => count(groups, "group", "groups"),
		(groups, entries) => format!("{}, {}", count(groups, "group", "groups"), count(entries, "entry", "entries")),
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
	let help_lines = wrap_help_items(help_items(app.slim_mode, esc_action(app)), help_width);
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

	// Search results come from all groups, so they show their group instead of the date.
	let show_groups = !app.groups.is_empty() && !app.query.is_empty();
	let group_style = Style::new().fg(app.theme.accent).bold();
	let (groups, entries) = (&app.groups, &app.entries);
	let group_rows = app.group_rows.iter().map(|&index| {
		let group = &groups[index];
		let name = Span::styled(format!("{}/", group.name), group_style);
		if app.slim_mode {
			Row::new([Cell::from(name)])
		} else {
			Row::new([Cell::from(name), Cell::from(group_contents(groups, entries, group.id)), Cell::from("")])
		}
	});
	let entry_rows = app.filtered.iter().map(|&i| {
		let entry = &entries[i];
		let user = masked_user(entry, &app.theme);
		if app.slim_mode {
			Row::new([Cell::from(slim_row(entry, &app.theme))])
		} else if show_groups {
			Row::new([Cell::from(entry.name.as_str()), Cell::from(user), Cell::from(crate::groups::path(groups, entry.group))])
		} else {
			Row::new([Cell::from(entry.name.as_str()), Cell::from(user), Cell::from(entry.date_last_modify.as_str())])
		}
	});
	let rows = group_rows.chain(entry_rows);

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
		let last = if show_groups { "Group" } else { "Modified" };
		table_index = table_index.header(Row::new(["Name", "User", last]).style(Style::new().bold().fg(app.theme.header)));
	}
	let index_area = vertical[1];
	let header_height = header_height(app.slim_mode);
	app.index_page_size = index_area.height.saturating_sub(header_height).max(1) as usize;
	frame.render_stateful_widget(table_index, index_area, &mut app.index_state);

	let query_area = vertical[0];
	let fields = if app.search_notes { "[Name/User/Notes]" } else { "[Name/User]" };
	let title = if app.groups.is_empty() {
		format!("Index {fields} ")
	} else if app.query.is_empty() {
		format!("Index {} {fields} ", app.group_path(app.group))
	} else {
		format!("Index {fields} in all groups ")
	};
	let section = super::section(&title, &app.theme, 0).padding(Padding::ZERO);
	let field = section.inner(query_area);
	app.max_len = usize::from(field.width).min(crate::input::SECRET_CAP);
	let input_text = app.query.clone();
	let input = Paragraph::new(input_text.as_str());
	let cursor_column = UnicodeWidthStr::width(input_text.as_str());
	let cursor_x = field.x.saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX).min(field.width.saturating_sub(1)));
	frame.set_cursor_position((cursor_x, field.y));
	super::draw_input(frame, query_area, section, input, &app.theme);
}
