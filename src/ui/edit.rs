use ratatui::{
	Frame,
	layout::{Constraint, Layout, Margin, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{Clear, List, ListItem, Paragraph},
};
use std::ops::Range;
use std::time::{Duration, Instant};
#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
use unicode_segmentation::UnicodeSegmentation;
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
const NOTES_INDEX: usize = 5;

fn nav_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode {
		&["[PgUpDn]", "[Enter]", "[^s]", "[Esc]"]
	} else {
		&["[PgUpDn] Scroll notes", "[Enter] Edit field", "[^s] Save", "[Esc] Close"]
	}
}

fn field_help_items(slim_mode: bool, multiline: bool) -> &'static [&'static str] {
	if slim_mode {
		if multiline { &["[^s]", "[Esc]"] } else { &["[Enter]", "[Esc]"] }
	} else {
		if multiline { &["[^s] Save field", "[Esc] Cancel"] } else { &["[Enter] Save field", "[Esc] Cancel"] }
	}
}

fn split_edit_area(area: Rect, help_line_count: usize) -> (Rect, Rect) {
	let help_height = u16::try_from(help_line_count).unwrap_or(u16::MAX);
	let [body, help] = Layout::vertical([Constraint::Fill(1), Constraint::Length(help_height)]).areas(area);
	(body, help)
}

#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
pub(crate) fn field_at_position(area: Rect, slim_mode: bool, column: f64, row: u16) -> Option<(usize, bool)> {
	let help_lines = wrap_help_items(nav_help_items(slim_mode), area.width);
	let (body, _) = split_edit_area(area, help_lines.len());
	let inner = body.inner(Margin { horizontal: 0, vertical: 1 });
	let field = usize::from(row.checked_sub(inner.y)?).min(NOTES_INDEX);
	if row >= body.bottom() || !(f64::from(inner.x)..f64::from(inner.right())).contains(&column) {
		return None;
	}
	let label_end = if field == NOTES_INDEX { inner.x.saturating_add("Notes:".width() as u16) } else { value_area(inner, field).x };
	let on_label = row == inner.y.saturating_add(field as u16) && column < f64::from(label_end);
	Some((field, on_label))
}

/// Geometry of the text currently visible in the editor. These byte ranges
/// refer to its existing buffer and never contain copies of field contents.
pub(crate) struct EditView {
	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	area: Rect,
	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	rows: Vec<Range<usize>>,
	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	pub(crate) highlights: Vec<Rect>,
}

impl EditView {
	pub(super) fn new(area: Rect, rows: impl Iterator<Item = Range<usize>>, text: &str, selection: Option<&Range<usize>>) -> Self {
		#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
		{
			let rows: Vec<_> = rows.collect();
			let highlights = selection.map_or_else(Vec::new, |selection| {
				rows.iter()
					.enumerate()
					.filter_map(|(row, visible)| selection_rect(area, row as u16, text, visible, selection))
					.collect()
			});
			Self { area, rows, highlights }
		}
		#[cfg(not(all(feature = "gui", target_os = "linux", target_env = "gnu")))]
		{
			let _ = (area, rows, text, selection);
			Self {}
		}
	}

	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	pub(crate) fn line_at(&self, row: f64) -> Option<&Range<usize>> {
		let row = row - f64::from(self.area.y);
		if !(0.0..f64::from(self.area.height)).contains(&row) {
			return None;
		}
		self.rows.get(row as usize)
	}

	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	pub(crate) fn offset_at(&self, text: &str, column: f64, row: f64, dragging: bool) -> Option<usize> {
		if self.area.is_empty() {
			return None;
		}
		let x = column - f64::from(self.area.x);
		let y = row - f64::from(self.area.y);
		if !dragging && (!(0.0..f64::from(self.area.width)).contains(&x) || !(0.0..f64::from(self.area.height)).contains(&y)) {
			return None;
		}
		let Some(range) = self.rows.get(y.max(0.0) as usize) else {
			return self.rows.last().map(|range| range.end);
		};
		let line = text.get(range.clone())?;
		let mut left = 0.0;
		for (byte, cluster) in line.grapheme_indices(true) {
			let width = cluster.width() as f64;
			if x < left + width / 2.0 {
				return Some(range.start + byte);
			}
			left += width;
		}
		Some(range.end)
	}
}

#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
fn selection_rect(area: Rect, row: u16, text: &str, visible: &Range<usize>, selection: &Range<usize>) -> Option<Rect> {
	let start = visible.start.max(selection.start);
	let end = visible.end.min(selection.end);
	let includes_newline = selection.contains(&visible.end) && matches!(text.as_bytes().get(visible.end), Some(b'\n' | b'\r'));
	if start > end || (start == end && !includes_newline) {
		return None;
	}

	let col = text.get(visible.start..start)?.width();
	// A selected newline fills through the end of its displayed row.
	let right = if includes_newline {
		usize::from(area.width)
	} else {
		text.get(visible.start..end)?.width().min(usize::from(area.width))
	};
	(col < right).then(|| Rect::new(area.x + col as u16, area.y + row, (right - col) as u16, 1))
}

fn value_area(inner: Rect, field: usize) -> Rect {
	let label = match field {
		NAME_INDEX => "Name: ",
		USER_INDEX => "User: ",
		PASSWORD_INDEX => "Password: ",
		URL_INDEX => "URL: ",
		TOTP_INDEX => "TOTP: ",
		_ => "",
	};
	let offset = label.width() as u16;
	Rect {
		x: inner.x.saturating_add(offset),
		y: inner.y.saturating_add(field as u16),
		width: inner.width.saturating_sub(offset),
		height: 1,
	}
}

fn notes_area(body: Rect, inner: Rect) -> Rect {
	let y = inner.y.saturating_add(NOTES_INDEX as u16 + 1);
	Rect { x: inner.x, y, width: inner.width, height: body.bottom().saturating_sub(y) }
}

#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
pub(crate) fn readonly_view<'a>(area: Rect, app: &'a App, field: usize, selection: Option<&Range<usize>>) -> Option<(EditView, &'a str)> {
	let entry = app.edit_entry.as_ref()?;
	let text = match field {
		NAME_INDEX => entry.name.as_str(),
		USER_INDEX => entry.user.as_str(),
		PASSWORD_INDEX => entry.password.as_str(),
		URL_INDEX => entry.url.as_str(),
		TOTP_INDEX => app.totp_cache.as_ref()?.code.as_str(),
		NOTES_INDEX => entry.notes.as_str(),
		_ => return None,
	};
	if text.is_empty() {
		return None;
	}

	let help_lines = wrap_help_items(nav_help_items(app.slim_mode), area.width);
	let (body, _) = split_edit_area(area, help_lines.len());
	let inner = body.inner(Margin { horizontal: 0, vertical: 1 });
	let view = if field == NOTES_INDEX {
		let area = notes_area(body, inner);
		let ranges = wrap_ranges(text, area.width.saturating_sub(1) as usize);
		let height = usize::from(area.height);
		let scroll = app.field_editor_scroll.min(ranges.len().saturating_sub(height));
		EditView::new(area, ranges.into_iter().skip(scroll).take(height), text, selection)
	} else {
		let area = value_area(inner, field);
		let mut width = 0;
		let mut end = 0;
		for cluster in text.graphemes(true) {
			width += cluster.width();
			if width > usize::from(area.width) || cluster.chars().any(char::is_control) {
				break;
			}
			end += cluster.len();
		}
		EditView::new(area, std::iter::once(0..end), text, selection)
	};
	Some((view, text))
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

pub fn draw_edit(frame: &mut Frame, app: &mut App) -> Option<EditView> {
	let full_area = frame.area();
	let entry = app.edit_entry.as_ref()?;
	let mut edit_view = None;

	let name = entry.name.as_str();
	let user = entry.user.as_str();
	let url = entry.url.as_str();
	let totp_raw = entry.totp.as_str();
	let notes = entry.notes.as_str();
	let last_modified = entry.date_last_modify.as_str();
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
	let help_items = if editing_field { field_help_items(app.slim_mode, selected == NOTES_INDEX) } else { nav_help_items(app.slim_mode) };
	let help_width = full_area.width;
	let help_lines = wrap_help_items(help_items, help_width);
	let (list_area, help_area) = split_edit_area(full_area, help_lines.len());
	let inner = list_area.inner(Margin { horizontal: 0, vertical: 1 });
	let notes_label_y = inner.y.saturating_add(NOTES_INDEX as u16);
	let notes_label_area = Rect { x: inner.x, y: notes_label_y, width: inner.width, height: 1 };
	let notes_area = notes_area(list_area, inner);
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
		compact_field(selected, NAME_INDEX, "Name", name, vec![], true, styles),
		compact_field(selected, USER_INDEX, "User", user, vec![], true, styles),
		compact_field(selected, PASSWORD_INDEX, "Password", "", vec![], !password_is_set, styles),
		compact_field(selected, URL_INDEX, "URL", url, vec![], true, styles),
		compact_field(selected, TOTP_INDEX, "TOTP", "", totp_extra, totp_raw.is_empty(), styles),
	];
	let title = if is_new_entry {
		"New Entry ".to_owned()
	} else if last_modified.is_empty() {
		"Entry ".to_owned()
	} else {
		format!("Entry - modified {last_modified} ")
	};
	let list = List::new(items).block(super::section(&title, theme, 0));
	let mut list_state = app.edit_state;
	if selected == NOTES_INDEX {
		list_state.select(None);
	} else {
		list_state.select(Some(selected));
	}
	frame.render_stateful_widget(list, list_area, &mut list_state);

	// Render the password separately from the List.
	if password_is_set {
		let field_area = value_area(inner, PASSWORD_INDEX);
		frame.render_widget(Paragraph::new(password).style(normal), field_area);
	}

	// Render the current TOTP code separately from the List.
	// The otpauth URI/secret is deliberately not rendered while viewing.
	if let Some((totp_view, _)) = totp_code.as_ref() {
		let field_area = value_area(inner, TOTP_INDEX);
		frame.render_widget(Paragraph::new(*totp_view).style(normal.bold()), field_area);
	}

	// Overlay the single-line field editor when editing
	// Name/User/Password/URL/TOTP.
	if editing_field
		&& selected <= TOTP_INDEX
		&& let Some(editor) = app.field_editor.as_ref()
	{
		let field_area = value_area(inner, selected);
		frame.render_widget(Clear, field_area);

		let available_width = field_area.width as usize;
		let (visible, cursor_column) = editor.single_line_range(available_width);
		let text = editor.text();
		frame.render_widget(Paragraph::new(&text[visible.clone()]).style(super::input_style(theme)), field_area);
		let selection = editor.selection();
		edit_view = Some(EditView::new(field_area, std::iter::once(visible), text, selection.as_ref()));

		let cursor_x = field_area.x.saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX));
		if selection.is_none()
			&& cursor_column < available_width
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
			} else if cursor_row >= app.field_editor_scroll.saturating_add(visible_height) {
				app.field_editor_scroll = cursor_row - visible_height + 1;
			}
			let visible_lines = ranges
				.iter()
				.skip(app.field_editor_scroll)
				.take(visible_height)
				.map(|r| Line::from(Span::styled(&text[r.clone()], normal)))
				.collect::<Vec<_>>();
			frame.render_widget(Paragraph::new(visible_lines).style(super::input_style(theme)), notes_area);
			let visible = ranges.into_iter().skip(app.field_editor_scroll).take(visible_height);
			let selection = editor.selection();
			edit_view = Some(EditView::new(notes_area, visible, text, selection.as_ref()));
			let visible_row = cursor_row.saturating_sub(app.field_editor_scroll);
			if selection.is_none() && cursor_col < notes_area.width as usize && visible_row < notes_area.height as usize {
				let x = notes_area.x.saturating_add(u16::try_from(cursor_col).unwrap_or(u16::MAX));
				let y = notes_area.y.saturating_add(u16::try_from(visible_row).unwrap_or(u16::MAX));
				if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
					cell.set_style(accent_style);
				}
			}
		}
	} else {
		let visible_lines = notes_lines.into_iter().skip(app.field_editor_scroll).take(notes_area.height as usize).collect::<Vec<_>>();
		frame.render_widget(Paragraph::new(visible_lines), notes_area);
	}
	let help = if let Some(status) = &app.status {
		Paragraph::new(status.as_str()).style(Style::new().fg(app.theme.warning))
	} else {
		let help_text = help_lines.join("\n");
		Paragraph::new(help_text).style(Style::new().fg(app.theme.accent))
	};
	frame.render_widget(help, help_area);
	edit_view
}

#[cfg(all(test, feature = "gui", target_os = "linux", target_env = "gnu"))]
mod tests {
	use super::*;

	#[test]
	fn mouse_hit_testing_and_highlight_follow_visible_graphemes() {
		let text = "น้ำ
猫z";
		let rows = wrap_ranges(text, 10);
		let area = Rect::new(3, 4, 10, 3);
		let second = "น้ำ
"
		.len();
		let selection = 0..second + "猫".len();
		let view = EditView::new(area, rows.into_iter(), text, Some(&selection));

		assert_eq!(view.offset_at(text, 2.9, 4.0, false), None);
		assert_eq!(view.offset_at(text, 3.4, 4.0, false), Some(0));
		assert_eq!(view.offset_at(text, 4.6, 4.0, false), Some("น้ำ".len()));
		assert_eq!(view.offset_at(text, 4.5, 5.0, true), Some(second + "猫".len()));
		assert_eq!(view.offset_at(text, 3.0, 6.0, true), Some(text.len()));
		assert_eq!(view.highlights, [Rect::new(3, 4, 10, 1), Rect::new(3, 5, 2, 1)]);
	}
}
