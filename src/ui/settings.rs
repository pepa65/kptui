use ratatui::{
	Frame,
	layout::{Alignment, Constraint, Direction, Layout, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{Block, Borders, List, ListItem, Padding, Paragraph},
};

use crate::app::App;
use crate::input::settings::{AUTO_LOCK_ROW, DATABASE_ROW, KEYFILE_ROW, THEME_ROW};
use crate::util::wrap_help_items;

fn centered_cursor_x(input_area: Rect, typed_len: u16) -> u16 {
	let inner = Block::default().borders(Borders::ALL).inner(input_area);
	inner.x + (inner.width / 2).saturating_sub(typed_len / 2) + typed_len
}

fn nav_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode {
		&["[↑↓]", "[Enter]", "[Esc]"]
	} else {
		&["[↑↓] navigate", "[Enter] edit / choose theme", "[Esc] back"]
	}
}

fn field_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] save", "[Esc] cancel"] }
}

fn change_password_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] confirm", "[Esc] cancel"] }
}

fn export_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] continue", "[Esc] cancel"] }
}

fn import_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[Enter] continue", "[Esc] cancel"] }
}

pub fn draw_settings(frame: &mut Frame, app: &mut App) {
	if app.exporting_database {
		draw_export(frame, app);
	} else if app.importing_database {
		draw_import(frame, app);
	} else if app.changing_password {
		draw_change_password(frame, app);
	} else if app.choosing_theme {
		draw_theme_picker(frame, app);
	} else {
		draw_main_settings(frame, app);
	}
}

fn draw_main_settings(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let editing_field = app.editing_field;
	let selected = app.settings_state.selected().unwrap_or(0);

	let help_items = if editing_field { field_help_items(app.slim_mode) } else { nav_help_items(app.slim_mode) };

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(help_items, help_width);
	let help_height = help_lines.len() as u16;

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let theme = &app.theme;
	let normal = Style::new().fg(theme.text);
	let accent = Style::new().fg(theme.accent);
	let warning = Style::new().fg(theme.warning);
	let border_style = Style::new().fg(theme.border);
	let placeholder = Style::new().fg(theme.text).bold();
	let editing_style = Style::new().fg(theme.selection_fg).bg(theme.selection_bg);
	let label_style = Style::new().fg(theme.header).bold();

	let field = |field_index: usize, label: &'static str, value: String| -> ListItem<'static> {
		let current_label_style = if selected == field_index { editing_style } else { label_style };

		let value_line = if value.is_empty() {
			Line::from(Span::styled("(not set)", placeholder))
		} else {
			Line::from(Span::styled(value, normal))
		};

		ListItem::new(vec![Line::from(Span::styled(label, current_label_style)), value_line, Line::from("")])
	};

	let database_value = app.config.default_database.as_ref().map(|p| p.display().to_string()).unwrap_or_default();

	let keyfile_value = app.config.keyfile.as_ref().map(|p| p.display().to_string()).unwrap_or_default();

	let auto_lock_value = app.config.auto_lock.as_secs().to_string();

	let theme_value = if app.available_themes.is_empty() {
		Line::from(Span::styled("no themes found in ~/.config/kptui/themes", warning))
	} else {
		match app.config.theme.as_deref() {
			Some(name) => Line::from(Span::styled(crate::theme::slugify(name), normal)),
			None => Line::from(Span::styled("(not set)", placeholder)),
		}
	};

	let items = vec![
		field(DATABASE_ROW, "Default database", database_value),
		field(KEYFILE_ROW, "Keyfile (optional)", keyfile_value),
		field(AUTO_LOCK_ROW, "Auto-lock (seconds)", auto_lock_value),
		ListItem::new(vec![
			Line::from(Span::styled("Theme", if selected == THEME_ROW { editing_style } else { label_style })),
			theme_value,
			Line::from(""),
		]),
		ListItem::new(vec![
			Line::from(Span::styled("Change master password", if selected == 4 { editing_style } else { label_style })),
			Line::from(Span::styled("[Enter] to change", placeholder)),
			Line::from(""),
		]),
		ListItem::new(vec![
			Line::from(Span::styled("Import database", if selected == 5 { editing_style } else { label_style })),
			Line::from(Span::styled("[Enter] to import", placeholder)),
			Line::from(""),
		]),
		ListItem::new(vec![
			Line::from(Span::styled("Export database", if selected == 6 { editing_style } else { label_style })),
			Line::from(Span::styled("[Enter] to export", placeholder)),
			Line::from(""),
		]),
	];

	let list = List::new(items).block(Block::default().borders(Borders::ALL).padding(Padding::horizontal(1)).border_style(border_style).title(" Settings "));

	frame.render_stateful_widget(list, vertical[0], &mut app.settings_state);

	// Overlay the FieldEditor on the selected editable setting.
	if editing_field
		&& selected < 3
		&& let Some(editor) = app.field_editor.as_ref()
	{
		let list_area = vertical[0];

		// The List has a one-cell border and one-cell horizontal padding.
		// Each setting occupies three rows:
		//   label
		//   value/editor
		//   blank
		let value_y = list_area.y + 2 + selected as u16 * 3;

		let editor_x = list_area.x + 2;
		let editor_width = list_area.width.saturating_sub(3);

		if editor_width > 0 {
			let (visible, cursor_column) = editor.single_line_view(editor_width as usize);

			let field_area = Rect { x: editor_x, y: value_y, width: editor_width, height: 1 };

			frame.render_widget(Paragraph::new(visible).style(normal), field_area);

			let cursor_x = field_area.x + cursor_column as u16;
			frame.set_cursor_position((cursor_x, field_area.y));
		}
	}

	let help = if let Some(status) = &app.status {
		Paragraph::new(format!("  {status}")).style(warning)
	} else {
		let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");

		Paragraph::new(help_text).style(accent)
	};

	frame.render_widget(help, vertical[1]);
}

fn draw_change_password(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(change_password_help_items(app.slim_mode), help_width);
	let help_height = help_lines.len() as u16;

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
	app.max_len = (input_area.width - 2) as usize;

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

	let input = Paragraph::new("•".repeat(buf.chars().count()))
		.alignment(Alignment::Center)
		.block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(app.theme.border)).title(title));

	frame.render_widget(input, input_area);

	let hint_area = Rect { x: input_area.x, y: input_area.y + input_area.height, width: input_area.width, height: 1 };

	let default_hint = match app.password_change_step {
		crate::app::PasswordChangeStep::Current => "verify current master password",
		crate::app::PasswordChangeStep::New | crate::app::PasswordChangeStep::ConfirmNew => "this re-encrypts the vault with the new password",
	};
	let hint_text = app.status.clone().unwrap_or_else(|| default_hint.to_string());
	let hint_style = if app.status.is_some() { Style::new().fg(app.theme.error) } else { Style::new().fg(app.theme.warning) };

	let hint = Paragraph::new(hint_text).alignment(Alignment::Center).style(hint_style);

	frame.render_widget(hint, hint_area);

	let typed_len = buf.chars().count() as u16;
	let cursor_x = centered_cursor_x(input_area, typed_len);
	frame.set_cursor_position((cursor_x, input_area.y + 1));

	let accent = Style::new().fg(app.theme.accent);
	let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
	frame.render_widget(Paragraph::new(help_text).style(accent), vertical[3]);
}

fn draw_import(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(import_help_items(app.slim_mode), help_width);
	let help_height = help_lines.len() as u16;

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
	app.max_len = (input_area.width - 2) as usize;

	let masked = matches!(app.import_step, crate::app::ImportStep::KdbxPassword);

	let title = match app.import_step {
		crate::app::ImportStep::Path => " Import from file (.kdbx / .csv / .json) ",
		crate::app::ImportStep::KdbxPassword => " Password for that vault ",
	};

	let buf = match app.import_step {
		crate::app::ImportStep::Path => &app.import_path_buffer,
		crate::app::ImportStep::KdbxPassword => &app.import_kdbx_password_buffer,
	};

	let display_text = if masked { "•".repeat(buf.chars().count()) } else { buf.clone() };

	let input = Paragraph::new(display_text)
		.alignment(Alignment::Center)
		.block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(app.theme.border)).title(title));

	frame.render_widget(input, input_area);

	let hint_area = Rect { x: input_area.x, y: input_area.y + input_area.height, width: input_area.width, height: 1 };

	let default_hint = match app.import_step {
		crate::app::ImportStep::Path => "only Name, User, Password, URL, TOTP and Notes are imported",
		crate::app::ImportStep::KdbxPassword => "that file's master password (not for this vault!)",
	};
	let hint_text = app.status.clone().unwrap_or_else(|| default_hint.to_string());
	let hint_style = if app.status.is_some() { Style::new().fg(app.theme.error) } else { Style::new().fg(app.theme.warning) };

	let hint = Paragraph::new(hint_text).alignment(Alignment::Center).style(hint_style);

	frame.render_widget(hint, hint_area);

	let typed_len = buf.chars().count() as u16;
	let cursor_x = centered_cursor_x(input_area, typed_len);
	frame.set_cursor_position((cursor_x, input_area.y + 1));

	let accent = Style::new().fg(app.theme.accent);
	let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
	frame.render_widget(Paragraph::new(help_text).style(accent), vertical[3]);
}

fn draw_export(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(export_help_items(app.slim_mode), help_width);
	let help_height = help_lines.len() as u16;

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
	app.max_len = (input_area.width - 2) as usize;

	let title = match app.export_step {
		crate::app::ExportStep::Path => " Export to file (.kdbx / .csv / .json) ",
		crate::app::ExportStep::Confirm => " Confirm export ",
	};

	let input = Paragraph::new(app.export_path_buffer.clone())
		.alignment(Alignment::Center)
		.block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(app.theme.border)).title(title));

	frame.render_widget(input, input_area);

	let hint_area = Rect { x: input_area.x, y: input_area.y + input_area.height, width: input_area.width, height: 1 };

	let default_hint = match app.export_step {
		crate::app::ExportStep::Path => "only Name, User, Password, URL, and TOTP are exported".to_string(),
		crate::app::ExportStep::Confirm => {
			let path = app.pending_export_path.as_deref();
			let plaintext = path
				.and_then(|p| crate::export::detect_format(p).ok())
				.map(|f| matches!(f, crate::export::ExportFormat::Csv | crate::export::ExportFormat::Json))
				.unwrap_or(false);
			let overwrite = path.map(|p| p.exists()).unwrap_or(false);

			match (plaintext, overwrite) {
				(true, true) => "overwriting an existing file with all passwords in PLAINTEXT — press Enter to confirm".to_string(),
				(true, false) => "writing all passwords in PLAINTEXT to disk — press Enter to confirm".to_string(),
				(false, true) => "overwriting an existing file — press Enter to confirm".to_string(),
				(false, false) => "press Enter to confirm".to_string(),
			}
		}
	};
	let hint_text = app.status.clone().unwrap_or(default_hint);
	let hint_style = if app.status.is_some() { Style::new().fg(app.theme.error) } else { Style::new().fg(app.theme.warning) };

	let hint = Paragraph::new(hint_text).alignment(Alignment::Center).style(hint_style);

	frame.render_widget(hint, hint_area);

	let typed_len = app.export_path_buffer.chars().count() as u16;
	let cursor_x = centered_cursor_x(input_area, typed_len);
	frame.set_cursor_position((cursor_x, input_area.y + 1));

	let accent = Style::new().fg(app.theme.accent);
	let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
	frame.render_widget(Paragraph::new(help_text).style(accent), vertical[3]);
}

fn draw_theme_picker(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(export_help_items(app.slim_mode), help_width);
	let help_height = help_lines.len() as u16;

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	let theme = &app.theme;
	let normal = Style::new().fg(theme.text);
	let accent = Style::new().fg(theme.accent);
	let warning = Style::new().fg(theme.warning);
	let border_style = Style::new().fg(theme.border);
	let current_theme = app.config.theme.as_deref().map(crate::theme::slugify);

	let items: Vec<ListItem> = app
		.available_themes
		.iter()
		.map(|name| {
			let is_active = current_theme.as_deref().is_some_and(|current| current == crate::theme::slugify(name));

			let marker = if is_active { " ◉" } else { " ○" };
			let marker_style = if is_active { accent } else { normal };

			ListItem::new(Line::from(vec![Span::styled(format!("{marker} "), marker_style), Span::styled(name.clone(), normal)]))
		})
		.collect();

	let list = List::new(items)
		.block(Block::default().borders(Borders::ALL).padding(Padding::horizontal(1)).border_style(border_style).title(" Theme "))
		.highlight_style(Style::new().fg(theme.selection_fg).bg(theme.selection_bg).bold());

	frame.render_stateful_widget(list, vertical[0], &mut app.theme_state);

	let help = if let Some(status) = &app.status {
		Paragraph::new(format!("  {status}")).style(warning)
	} else {
		let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");
		Paragraph::new(help_text).style(accent)
	};

	frame.render_widget(help, vertical[1]);
}
