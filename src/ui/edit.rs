use ratatui::{
	Frame,
	layout::{Constraint, Direction, Layout, Margin, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph},
};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, TotpCache};
use crate::db::current_totp_code;
use crate::input::fieldview::{cursor_position, wrap_ranges};
use crate::util::wrap_help_items;

const NAME_INDEX: usize = 0;
const USER_INDEX: usize = 1;
const PASSWORD_INDEX: usize = 2;
const URL_INDEX: usize = 3;
const TOTP_INDEX: usize = 4;
const LAST_MODIFIED_INDEX: usize = 5;
const NOTES_INDEX: usize = 6;

fn nav_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode {
		&["[↑↓]", "[PgUpDn]", "[Enter]", "[^s]", "[Esc]"]
	} else {
		&["[↑↓] Navigate", "[PgUpDn] Scroll notes", "[Enter] Edit field", "[^s] Save", "[Esc] Close"]
	}
}

fn field_help_items(slim_mode: bool, multiline: bool) -> &'static [&'static str] {
	if slim_mode {
		if multiline { &["[^s]", "[Esc]"] } else { &["[Enter]", "[Esc]"] }
	} else {
		if multiline { &["[^s] Save field", "[Esc] Cancel"] } else { &["[Enter] Save field", "[Esc] Cancel"] }
	}
}

fn compact_field<'a>(
	selected: usize,
	field_index: usize,
	label: &'static str,
	value: &'a str,
	extra: Vec<Span<'a>>,
	show_empty: bool,
	styles: [Style; 4], // [label, editing, normal, placeholder]
) -> ListItem<'a> {
	let [label_style, editing_style, normal, placeholder] = styles;
	let label_style = if selected == field_index { editing_style } else { label_style };
	let mut line = vec![Span::styled(label, label_style), Span::styled(": ", normal)];
	if value.is_empty() && extra.is_empty() {
		if show_empty {
			line.push(Span::styled("(empty)", placeholder));
		}
	} else {
		line.push(Span::styled(value, normal));
		line.extend(extra);
	}
	ListItem::new(Line::from(line))
}

pub fn draw_edit(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();
	let Some(entry) = app.edit_entry.as_ref() else {
		return;
	};

	let name = entry.name.clone();
	let user = entry.user.as_str();
	let url = entry.url.clone();
	let totp_raw = entry.totp.as_str();
	let notes = entry.notes.as_str();
	let last_modified = entry.date_last_modify.clone();
	let password = entry.password.as_str();
	let password_is_set = !password.is_empty();
	let is_new_entry = app.edit_target.is_none();
	let editing_field = app.editing_field;
	let selected = app.edit_state.selected().unwrap_or(0);
	let totp_active = !(editing_field && selected == TOTP_INDEX);
	if totp_active && app.totp_cache.as_ref().is_none_or(|c| c.expires <= Instant::now()) {
		app.totp_cache = current_totp_code(totp_raw).map(|t| TotpCache { code: t.code, expires: Instant::now() + t.valid_for });
	}
	let totp_code: Option<(&str, Duration)> = if totp_active {
		app.totp_cache.as_ref().map(|c| (c.code.as_str(), c.expires.saturating_duration_since(Instant::now())))
	} else {
		None
	};
	let theme = &app.theme;
	let label_style = Style::new().fg(theme.header).bold();
	let normal = Style::new().fg(theme.text);
	let warning = Style::new().fg(theme.warning);
	let placeholder = Style::new().fg(theme.text).bold();
	let editing_style = Style::new().fg(theme.selection_fg).bg(theme.selection_bg);
	let accent_style = Style::new().fg(theme.selection_fg).bg(theme.accent);
	let border_style = Style::new().fg(theme.border);
	let help_items = if editing_field { field_help_items(app.slim_mode, selected == NOTES_INDEX) } else { nav_help_items(app.slim_mode) };
	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(help_items, help_width);
	let help_height = help_lines.len() as u16;
	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);
	let list_area = vertical[0];
	let inner = list_area.inner(Margin { horizontal: 1, vertical: 1 });
	let notes_label_y = inner.y + 6;
	let notes_label_area = Rect { x: inner.x + 1, y: notes_label_y, width: inner.width.saturating_sub(1), height: 1 };
	let notes_area = Rect {
		x: inner.x + 1,
		y: notes_label_y + 1,
		width: inner.width.saturating_sub(1),
		height: inner.height.saturating_sub(7).max(1),
	};
	let notes_width = notes_area.width.saturating_sub(1) as usize;
	let notes_ranges = if notes.is_empty() { Vec::new() } else { wrap_ranges(notes, notes_width) };
	let notes_lines: Vec<Line> = if notes.is_empty() {
		vec![Line::from(Span::styled("(empty)", placeholder))]
	} else {
		notes_ranges.iter().map(|r| Line::from(Span::styled(&notes[r.clone()], normal))).collect()
	};
	let max_notes_scroll = notes_lines.len().saturating_sub(notes_area.height as usize);
	app.field_editor_scroll = app.field_editor_scroll.min(max_notes_scroll);
	let totp_extra = match totp_code.as_ref() {
		Some((_, valid_for)) => {
			vec![Span::styled(format!(" ({}s)", valid_for.as_secs() + 1), warning)]
		}
		None if totp_raw.is_empty() || (editing_field && selected == TOTP_INDEX) => {
			vec![]
		}
		None => {
			vec![Span::styled("couldn't generate a code", warning)]
		}
	};
	let styles = [label_style, editing_style, normal, placeholder];
	let items = vec![
		compact_field(selected, NAME_INDEX, "Name", &name, vec![], true, styles),
		compact_field(selected, USER_INDEX, "User", user, vec![], true, styles),
		compact_field(selected, PASSWORD_INDEX, "Password", "", vec![], !password_is_set, styles),
		compact_field(selected, URL_INDEX, "URL", &url, vec![], true, styles),
		compact_field(selected, TOTP_INDEX, "TOTP", "", totp_extra, totp_raw.is_empty(), styles),
		compact_field(selected, LAST_MODIFIED_INDEX, "Modified", &last_modified, vec![], true, styles),
	];
	let title = if is_new_entry { " New Entry " } else { " Entry " };
	let list = List::new(items).block(
		Block::default()
			.borders(Borders::ALL)
			.padding(Padding::horizontal(1))
			.border_style(border_style)
			.title(Span::styled(title, Style::default().fg(app.theme.error))),
	);
	let mut list_state = app.edit_state;
	if selected == NOTES_INDEX {
		list_state.select(None);
	} else {
		list_state.select(Some(selected));
	}
	frame.render_stateful_widget(list, list_area, &mut list_state);

	// Render the password separately from the List.
	if password_is_set {
		let label = "Password: ";
		let label_width = UnicodeWidthStr::width(label) as u16;
		let field_area = Rect {
			x: inner.x + label_width + 1,
			y: inner.y + PASSWORD_INDEX as u16,
			width: inner.width.saturating_sub(label_width + 1),
			height: 1,
		};
		frame.render_widget(Paragraph::new(password).style(normal), field_area);
	}

	// Render the current TOTP code separately from the List.
	// The otpauth URI/secret is deliberately not rendered while viewing.
	if let Some((totp_view, _)) = totp_code.as_ref() {
		let label = "TOTP: ";
		let label_width = UnicodeWidthStr::width(label) as u16;
		let field_area = Rect {
			x: inner.x + label_width + 1,
			y: inner.y + TOTP_INDEX as u16,
			width: inner.width.saturating_sub(label_width + 1),
			height: 1,
		};
		frame.render_widget(Paragraph::new(*totp_view).style(normal.bold()), field_area);
	}

	// Overlay the single-line field editor when editing
	// Name/User/Password/URL/TOTP.
	if editing_field
		&& selected <= TOTP_INDEX
		&& let Some(editor) = app.field_editor.as_ref()
	{
		let inner = vertical[0].inner(Margin { horizontal: 1, vertical: 1 });
		let label = match selected {
			NAME_INDEX => "Name: ",
			USER_INDEX => "User: ",
			PASSWORD_INDEX => "Password: ",
			URL_INDEX => "URL: ",
			TOTP_INDEX => "TOTP: ",
			_ => "",
		};
		let label_width = UnicodeWidthStr::width(label) as u16;
		let field_area = Rect {
			x: inner.x + label_width + 1,
			y: inner.y + selected as u16,
			width: inner.width.saturating_sub(label_width + 1),
			height: 1,
		};
		frame.render_widget(Clear, field_area);

		let available_width = field_area.width as usize;
		let (visible_text, cursor_column) = editor.single_line_view(available_width);
		frame.render_widget(Paragraph::new(visible_text.as_str()).style(normal), field_area);

		let cursor_x = field_area.x + cursor_column as u16;
		if cursor_x < field_area.x + field_area.width
			&& let Some(cell) = frame.buffer_mut().cell_mut((cursor_x, field_area.y))
		{
			cell.set_style(accent_style);
		}
	}
	let notes_label_style = if selected == NOTES_INDEX { editing_style } else { label_style };
	let notes_label = Line::from(vec![Span::styled("Notes", notes_label_style), Span::styled(":", normal)]);
	frame.render_widget(Paragraph::new(notes_label), notes_label_area);

	if editing_field && selected == NOTES_INDEX {
		if let Some(editor) = app.field_editor.as_ref() {
			frame.render_widget(Clear, notes_area);
			let text = editor.text();
			let ranges = wrap_ranges(text, notes_width);
			let (cursor_row, cursor_col) = cursor_position(text, &ranges, editor.cursor());
			let visible_height = notes_area.height as usize;
			if cursor_row < app.field_editor_scroll {
				app.field_editor_scroll = cursor_row;
			} else if cursor_row >= app.field_editor_scroll + visible_height {
				app.field_editor_scroll = cursor_row - visible_height + 1;
			}
			let visible_lines = ranges
				.iter()
				.skip(app.field_editor_scroll)
				.take(visible_height)
				.map(|r| Line::from(Span::styled(&text[r.clone()], normal)))
				.collect::<Vec<_>>();
			frame.render_widget(Paragraph::new(visible_lines), notes_area);
			let x = notes_area.x + cursor_col as u16;
			let y = notes_area.y + cursor_row.saturating_sub(app.field_editor_scroll) as u16;
			if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
				cell.set_style(accent_style);
			}
		}
	} else {
		let visible_lines = notes_lines.into_iter().skip(app.field_editor_scroll).take(notes_area.height as usize).collect::<Vec<_>>();
		frame.render_widget(Paragraph::new(visible_lines), notes_area);
	}
	let help = if let Some(status) = &app.status {
		Paragraph::new(format!("  {status}")).style(Style::new().fg(app.theme.warning))
	} else {
		let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
		Paragraph::new(help_text).style(Style::new().fg(app.theme.accent))
	};
	frame.render_widget(help, vertical[1]);
}
