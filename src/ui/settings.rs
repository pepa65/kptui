use std::borrow::Cow;

use ratatui::{
	Frame,
	layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{Clear, List, ListItem, Paragraph},
};

use super::edit::EditView;
use crate::app::{App, ExportStep, ImportStep};
use crate::input::fieldedit::FieldEditor;
use crate::input::settings::{AUTO_EXIT_ROW, LOCKED_ROW_COUNT, UNLOCKED_ROW_COUNT};
use crate::util::wrap_help_items;
use unicode_width::UnicodeWidthStr;

fn centered_cursor_x(input_area: Rect, typed_len: usize) -> u16 {
	let inner = input_area.inner(Margin { horizontal: 1, vertical: 1 });
	if inner.width == 0 {
		return inner.x;
	}

	let width = inner.width as usize;
	let offset = if typed_len < width { (width - typed_len) / 2 + typed_len } else { width - 1 };
	inner.x.saturating_add(offset.min(width - 1) as u16)
}

fn nav_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] Modify", "[Esc] Back"] }
}

fn field_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] Save", "[Esc] Cancel"] }
}

fn change_password_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] Confirm", "[Esc] Cancel"] }
}

fn export_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] Continue", "[Esc] Cancel"] }
}

fn import_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] Continue", "[Esc] Cancel"] }
}

pub fn draw_settings(frame: &mut Frame, app: &mut App) -> Option<EditView> {
	if app.exporting_database {
		return draw_export(frame, app);
	} else if app.importing_database {
		return draw_import(frame, app);
	} else if app.changing_keyfile {
		draw_change_keyfile(frame, app);
	} else if app.changing_password {
		draw_change_password(frame, app);
	} else if app.choosing_theme {
		draw_theme_picker(frame, app);
	} else {
		draw_main_settings(frame, app);
	}
	None
}

fn draw_main_settings(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let editing_field = app.editing_field;
	let selected = app.settings_state.selected().unwrap_or(0);

	let help_items = if editing_field { field_help_items(app.slim_mode) } else { nav_help_items(app.slim_mode) };

	let help_width = full_area.width;
	let help_lines = wrap_help_items(help_items, help_width);
	let help_height = u16::try_from(help_lines.len()).unwrap_or(u16::MAX);

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let theme = &app.theme;
	let normal = Style::new().fg(theme.text);
	let accent = Style::new().fg(theme.accent);
	let warning = Style::new().fg(theme.warning);
	let placeholder = Style::new().fg(theme.text).bold();
	let editing_style = Style::new().fg(theme.selection_fg).bg(theme.selection_bg);
	let label_style = Style::new().fg(theme.header).bold();
	const LABELS: [&str; UNLOCKED_ROW_COUNT] = [
		"Database",
		"Keyfile (optional)",
		"KDF iterations (time cost)",
		"Auto-exit (seconds)",
		"Theme",
		"Change master password",
		"Import data",
		"Export data",
	];

	let path_value =
		|path: Option<&std::path::Path>| path.map_or_else(|| Span::styled("(not set)", placeholder), |path| Span::styled(path.display().to_string(), normal));
	let theme_value = if app.available_themes.is_empty() {
		Span::styled("no themes found in ~/.config/kptui/themes", warning)
	} else if let Some(configured) = &app.config.theme {
		let name = app.available_themes.iter().find(|name| crate::theme::slugify(name) == *configured).unwrap_or(configured);
		Span::styled(name.as_str(), normal)
	} else {
		Span::styled("(not set)", placeholder)
	};
	let values = [
		path_value(app.config.default_database.as_deref()),
		path_value(app.config.keyfile.as_deref()),
		Span::styled(app.config.kdf_iterations.to_string(), normal),
		Span::styled(app.config.auto_exit.as_secs().to_string(), normal),
		theme_value,
		Span::styled("[Enter] to change", placeholder),
		Span::styled("[Enter] to import", placeholder),
		Span::styled("[Enter] to export", placeholder),
	];
	let row_count = if app.login_settings { LOCKED_ROW_COUNT } else { UNLOCKED_ROW_COUNT };
	let items = LABELS.into_iter().zip(values).take(row_count).enumerate().map(|(row, (label, value))| {
		let style = if selected == row { editing_style } else { label_style };
		ListItem::new(Line::from(vec![Span::styled(label, style), Span::styled(": ", normal), value]))
	});

	let section = super::section("Configuration ", theme, 0);
	let list_area = section.inner(vertical[0]);
	let list = List::new(items).block(section);
	frame.render_stateful_widget(list, vertical[0], &mut app.settings_state);

	if editing_field
		&& selected <= AUTO_EXIT_ROW
		&& let Some(editor) = app.field_editor.as_ref()
		&& let Some(row) = selected.checked_sub(app.settings_state.offset())
		&& row < usize::from(list_area.height)
	{
		let offset = (LABELS[selected].width() + 2) as u16;
		let field_area = Rect {
			x: list_area.x.saturating_add(offset),
			y: list_area.y.saturating_add(row as u16),
			width: list_area.width.saturating_sub(offset),
			height: 1,
		};
		if field_area.width > 0 {
			let (visible, cursor_column) = editor.single_line_view(usize::from(field_area.width));
			frame.render_widget(Clear, field_area);
			frame.render_widget(Paragraph::new(visible.as_str()).style(super::input_style(theme)), field_area);
			if cursor_column < usize::from(field_area.width) {
				frame.set_cursor_position((field_area.x.saturating_add(cursor_column as u16), field_area.y));
			}
		}
	}

	let help = if let Some(status) = &app.status {
		Paragraph::new(status.as_str()).style(warning)
	} else {
		let help_text = help_lines.join("\n");

		Paragraph::new(help_text).style(accent)
	};

	frame.render_widget(help, vertical[1]);
}

fn draw_change_password(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(change_password_help_items(app.slim_mode), help_width);
	let help_height = u16::try_from(help_lines.len()).unwrap_or(u16::MAX);

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(3), Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let input_row = vertical[1];

	let horizontal = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Fill(1), Constraint::Length(50), Constraint::Fill(1)])
		.split(input_row);

	let input_area = horizontal[1];
	app.max_len = input_area.width.saturating_sub(2) as usize;

	let title = match app.password_change_step {
		crate::app::PasswordChangeStep::Current => " Current master password ",
		crate::app::PasswordChangeStep::New => " New master password ",
		crate::app::PasswordChangeStep::ConfirmNew => " Confirm new master password ",
	};

	let buf = match app.password_change_step {
		crate::app::PasswordChangeStep::Current => &app.current_password_buffer,
		crate::app::PasswordChangeStep::New => &app.new_password_buffer,
		crate::app::PasswordChangeStep::ConfirmNew => &app.new_password_confirm,
	};

	let display = super::mask_password(buf);
	let typed_len = display.width();
	let input = Paragraph::new(display.as_str()).alignment(Alignment::Center);

	super::draw_input(frame, input_area, super::section(title, &app.theme, 1), input, &app.theme);

	let hint_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 1 };

	let default_hint = match app.password_change_step {
		crate::app::PasswordChangeStep::Current => "verify current master password",
		crate::app::PasswordChangeStep::New | crate::app::PasswordChangeStep::ConfirmNew => "this re-encrypts the vault with the new password",
	};
	let hint_text = app.status.as_deref().unwrap_or(default_hint);
	let hint_style = if app.status.is_some() { Style::new().fg(app.theme.error) } else { Style::new().fg(app.theme.warning) };

	let hint = Paragraph::new(hint_text).alignment(Alignment::Center).style(hint_style);

	frame.render_widget(hint, hint_area);

	let cursor_x = centered_cursor_x(input_area, typed_len);
	if input_area.height > 1 {
		frame.set_cursor_position((cursor_x, input_area.y.saturating_add(1)));
	}
	let accent = Style::new().fg(app.theme.accent);
	let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
	frame.render_widget(Paragraph::new(help_text).style(accent), vertical[3]);
}

fn draw_change_keyfile(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(change_password_help_items(app.slim_mode), help_width);
	let help_height = u16::try_from(help_lines.len()).unwrap_or(u16::MAX);

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(3), Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let horizontal = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Fill(1), Constraint::Length(60), Constraint::Fill(1)])
		.split(vertical[1]);

	let input_area = horizontal[1];
	app.max_len = input_area.width.saturating_sub(2) as usize;

	let (title, display) = match app.keyfile_change_step {
		crate::app::KeyfileChangeStep::Current => (" Current master password ", Cow::Owned(super::mask_password(&app.current_password_buffer))),
		crate::app::KeyfileChangeStep::Path => (" New keyfile path (empty to remove) ", Cow::Borrowed(app.keyfile_path_buffer.as_str())),
	};

	let typed_len = display.as_ref().width();
	let input = Paragraph::new(display.as_ref()).alignment(Alignment::Center);

	super::draw_input(frame, input_area, super::section(title, &app.theme, 1), input, &app.theme);

	let hint_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 1 };

	let default_hint = match app.keyfile_change_step {
		crate::app::KeyfileChangeStep::Current => "verify current master password using the current keyfile",
		crate::app::KeyfileChangeStep::Path => "re-encrypts the vault with the new keyfile",
	};

	let hint_text = app.status.as_deref().unwrap_or(default_hint);
	let hint_style = if app.status.is_some() { Style::new().fg(app.theme.error) } else { Style::new().fg(app.theme.warning) };

	frame.render_widget(Paragraph::new(hint_text).alignment(Alignment::Center).style(hint_style), hint_area);

	let cursor_x = centered_cursor_x(input_area, typed_len);
	if input_area.height > 1 {
		frame.set_cursor_position((cursor_x, input_area.y.saturating_add(1)));
	}

	let accent = Style::new().fg(app.theme.accent);
	let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");

	frame.render_widget(Paragraph::new(help_text).style(accent), vertical[3]);
}

fn draw_path_input(frame: &mut Frame, area: Rect, title: &str, editor: &FieldEditor, theme: &crate::theme::Theme) -> EditView {
	let section = super::section(title, theme, 1);
	let field_area = section.inner(area);
	let (visible, cursor_column) = editor.single_line_range(usize::from(field_area.width));
	let text = editor.text();
	super::draw_input(frame, area, section, Paragraph::new(&text[visible.clone()]), theme);
	let selection = editor.selection();
	if field_area.height > 0 && selection.is_none() && cursor_column < usize::from(field_area.width) {
		frame.set_cursor_position((field_area.x + cursor_column as u16, field_area.y));
	}
	EditView::new(field_area, std::iter::once(visible), text, selection.as_ref())
}

fn draw_import(frame: &mut Frame, app: &mut App) -> Option<EditView> {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(import_help_items(app.slim_mode), help_width);
	let help_height = u16::try_from(help_lines.len()).unwrap_or(u16::MAX);

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(3), Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let input_row = vertical[1];

	let horizontal = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Fill(1), Constraint::Length(60), Constraint::Fill(1)])
		.split(input_row);

	let input_area = horizontal[1];
	let view = match app.import_step {
		ImportStep::Path => app.field_editor.as_ref().map(|editor| {
			draw_path_input(frame, input_area, " Import from file (.kdbx/.csv/.json) ", editor, &app.theme)
		}),
		ImportStep::KdbxPassword => {
			let masked = super::mask_password(&app.import_kdbx_password_buffer);
			let cursor_x = centered_cursor_x(input_area, masked.width());
			let section = super::section(" Password for that vault ", &app.theme, 1);
			let input = Paragraph::new(masked).alignment(Alignment::Center);
			super::draw_input(frame, input_area, section, input, &app.theme);
			if input_area.height > 1 {
				frame.set_cursor_position((cursor_x, input_area.y.saturating_add(1)));
			}
			None
		}
	};

	let hint_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 1 };

	let default_hint = match app.import_step {
		ImportStep::Path => "only Name, User, Password, URL, TOTP and Notes are imported",
		ImportStep::KdbxPassword => "that file's master password (not for this vault!)",
	};
	let hint_text = app.status.as_deref().unwrap_or(default_hint);
	let hint_style = if app.status.is_some() { Style::new().fg(app.theme.error) } else { Style::new().fg(app.theme.warning) };

	let hint = Paragraph::new(hint_text).alignment(Alignment::Center).style(hint_style);

	frame.render_widget(hint, hint_area);

	let accent = Style::new().fg(app.theme.accent);
	let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
	frame.render_widget(Paragraph::new(help_text).style(accent), vertical[3]);
	view
}

fn draw_export(frame: &mut Frame, app: &mut App) -> Option<EditView> {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(export_help_items(app.slim_mode), help_width);
	let help_height = u16::try_from(help_lines.len()).unwrap_or(u16::MAX);

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(3), Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let input_row = vertical[1];

	let horizontal = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Fill(1), Constraint::Length(60), Constraint::Fill(1)])
		.split(input_row);

	let input_area = horizontal[1];
	let view = match app.export_step {
		ExportStep::Path => app.field_editor.as_ref().map(|editor| {
			draw_path_input(frame, input_area, " Export to file (.kdbx/.csv/.json) ", editor, &app.theme)
		}),
		ExportStep::Confirm => {
			let section = super::section(" Confirm export ", &app.theme, 1);
			let path = app.pending_export_path.as_deref().map(|path| path.to_string_lossy()).unwrap_or_default();
			frame.render_widget(Paragraph::new(path.as_ref()).block(section), input_area);
			None
		}
	};

	let hint_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 1 };

	let default_hint = match app.export_step {
		ExportStep::Path => "only Name, User, Password, URL, and TOTP are exported",
		ExportStep::Confirm => {
			let path = app.pending_export_path.as_deref();
			let plaintext = path
				.and_then(|p| crate::export::detect_format(p).ok())
				.map(|f| matches!(f, crate::export::ExportFormat::Csv | crate::export::ExportFormat::Json))
				.unwrap_or(false);
			let overwrite = path.map(|p| p.exists()).unwrap_or(false);

			match (plaintext, overwrite) {
				(true, true) => "overwriting an existing file with all passwords in PLAINTEXT — [Enter] confirm",
				(true, false) => "writing all passwords in PLAINTEXT to disk — [Enter] Confirm",
				(false, true) => "overwriting an existing file — [Enter] Confirm",
				(false, false) => "[Enter] Confirm",
			}
		}
	};
	let hint_text = app.status.as_deref().unwrap_or(default_hint);
	let hint_style = if app.status.is_some() { Style::new().fg(app.theme.error) } else { Style::new().fg(app.theme.warning) };

	let hint = Paragraph::new(hint_text).alignment(Alignment::Center).style(hint_style);

	frame.render_widget(hint, hint_area);

	let accent = Style::new().fg(app.theme.accent);
	let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
	frame.render_widget(Paragraph::new(help_text).style(accent), vertical[3]);
	view
}

fn draw_theme_picker(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let help_width = full_area.width;
	let help_lines = wrap_help_items(export_help_items(app.slim_mode), help_width);
	let help_height = u16::try_from(help_lines.len()).unwrap_or(u16::MAX);

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let theme = &app.theme;
	let normal = Style::new().fg(theme.text);
	let accent = Style::new().fg(theme.accent);
	let warning = Style::new().fg(theme.warning);
	let current_theme = app.config.theme.as_deref().map(crate::theme::slugify);

	let items: Vec<ListItem> = app
		.available_themes
		.iter()
		.map(|name| {
			let is_active = current_theme.as_deref().is_some_and(|current| current == crate::theme::slugify(name));

			let marker = if is_active { " ◉" } else { " ○" };
			let marker_style = if is_active { accent } else { normal };

			ListItem::new(Line::from(vec![Span::styled(format!("{marker} "), marker_style), Span::styled(name.as_str(), normal)]))
		})
		.collect();

	let list = List::new(items)
		.block(super::section("Theme ", theme, 0))
		.highlight_style(Style::new().fg(theme.selection_fg).bg(theme.selection_bg).bold());

	frame.render_stateful_widget(list, vertical[0], &mut app.theme_state);

	let help = if let Some(status) = &app.status {
		Paragraph::new(status.as_str()).style(warning)
	} else {
		let help_text = help_lines.join("\n");
		Paragraph::new(help_text).style(accent)
	};

	frame.render_widget(help, vertical[1]);
}
