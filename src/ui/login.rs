use std::borrow::Cow;

use ratatui::{
	Frame,
	layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{Paragraph, Wrap},
};

use crate::app::App;
use crate::input::login::database_missing;
use crate::util::default_new_database_path;

pub fn draw_login(frame: &mut Frame, app: &mut App) {
	let area = frame.area().inner(Margin { horizontal: 1, vertical: 1 });

	let logo_height = if app.slim_mode { 0 } else { 20 };

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(logo_height), Constraint::Length(1), Constraint::Length(3), Constraint::Fill(1)])
		.split(area);

	let logo_area = vertical[1];
	let filename_area = vertical[2];
	let input_row = vertical[3];

	let horizontal = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Fill(1), Constraint::Length(50), Constraint::Fill(1)])
		.split(input_row);

	let input_area = horizontal[1];

	// Calculate available input characters
	app.max_len = input_area.width.saturating_sub(2) as usize;

	let missing = database_missing(app);

	enum PromptContent<'a> {
		Masked(&'a str),
		Static(&'a str),
	}

	let version_title = app.db_version.as_ref().map(|v| format!(" {v} "));
	let (title, content) = if app.creating_database {
		if app.confirming_new_db_password {
			(" Confirm master password ", PromptContent::Masked(&app.new_db_confirm))
		} else {
			(" New master password ", PromptContent::Masked(&app.password))
		}
	} else if missing {
		(" Database file not present ", PromptContent::Static("[c] Create the new database"))
	} else {
		(version_title.as_deref().unwrap_or(""), PromptContent::Masked(&app.password))
	};

	let input_text: Cow<'_, str> = match content {
		PromptContent::Masked(buf) => Cow::Owned(super::mask_password(buf)),
		PromptContent::Static(text) => Cow::Borrowed(text),
	};
	let section = super::section(title, &app.theme, 1);
	let inner = section.inner(input_area);
	let input = Paragraph::new(input_text.as_ref()).alignment(Alignment::Center);
	if !app.slim_mode {
		let claws_style = Style::default().fg(app.theme.claws);
		let claws_light_style = Style::default().fg(app.theme.claws_light);
		let claws_shadow_style = Style::default().fg(app.theme.claws_shadow);
		let shell_style = Style::default().fg(app.theme.shell);
		let shell_light_style = Style::default().fg(app.theme.shell_light);
		let shell_shadow_style = Style::default().fg(app.theme.shell_shadow);

		let logo = Paragraph::new(vec![
			Line::from(vec![Span::styled("         ███", claws_light_style), Span::styled("                  ███         ", claws_style)]),
			Line::from(vec![Span::styled("        ██", claws_light_style), Span::styled("█        kptui       ███        ", claws_style)]),
			Line::from(vec![Span::styled("       ██", claws_light_style), Span::styled("█   █              █   ███       ", claws_style)]),
			Line::from(vec![
				Span::styled("      ██", claws_light_style),
				Span::styled("█  ██     ", claws_light_style),
				Span::styled(format!("v{:<8}", env!("CARGO_PKG_VERSION")), claws_light_style),
				Span::styled("  ██  ███      ", claws_style),
			]),
			Line::from(vec![Span::styled("      ██", claws_light_style), Span::styled("███                    █████      ", claws_style)]),
			Line::from(vec![
				Span::styled(" ██", claws_light_style),
				Span::styled("    ██          ", claws_style),
				Span::styled("█  █", claws_light_style),
				Span::styled("          ██    ██ ", claws_style),
			]),
			Line::from(vec![
				Span::styled("██", claws_light_style),
				Span::styled("█     ██         ", claws_style),
				Span::styled("█  █", claws_style),
				Span::styled("         ██     ███", claws_style),
			]),
			Line::from(vec![
				Span::styled("██", claws_light_style),
				Span::styled("██     ██  ", claws_style),
				Span::styled("████████████████", shell_light_style),
				Span::styled("  ██     ████", claws_style),
			]),
			Line::from(vec![
				Span::styled("  ██", claws_light_style),
				Span::styled("██     ", claws_style),
				Span::styled("██", shell_light_style),
				Span::styled("██████████████████", shell_style),
				Span::styled("     ████  ", claws_style),
			]),
			Line::from(vec![
				Span::styled("    ████  ", claws_style),
				Span::styled("██", shell_light_style),
				Span::styled("████████████████████", shell_style),
				Span::styled("  ████    ", claws_style),
			]),
			Line::from(vec![
				Span::styled("       ███", claws_style),
				Span::styled("██", shell_light_style),
				Span::styled("████████████████████", shell_style),
				Span::styled("███       ", claws_style),
			]),
			Line::from(vec![Span::styled("          ████████████████████", shell_style), Span::styled("██          ", shell_shadow_style)]),
			Line::from(vec![
				Span::styled("       ███", claws_style),
				Span::styled("████████████████████", shell_style),
				Span::styled("██", shell_shadow_style),
				Span::styled("███       ", claws_shadow_style),
			]),
			Line::from(vec![
				Span::styled("     ███  ██", claws_style),
				Span::styled("████████████████", shell_style),
				Span::styled("██", shell_shadow_style),
				Span::styled("██  ███     ", claws_shadow_style),
			]),
			Line::from(vec![
				Span::styled("    ██   ██  ", claws_style),
				Span::styled("████████████████", shell_shadow_style),
				Span::styled("  ██   ██    ", claws_shadow_style),
			]),
			Line::from(vec![Span::styled("     █  ██    ", claws_style), Span::styled("██          ██    ██   █    ", claws_shadow_style)]),
			Line::from(vec![Span::styled("         █   ", claws_style), Span::styled("██            ██    █        ", claws_shadow_style)]),
			Line::from(vec![Span::styled("              █            █              ", claws_shadow_style)]),
		])
		.alignment(Alignment::Center)
		.style(claws_style);
		frame.render_widget(logo, logo_area);
	}
	let default_path;
	let path = if let Some(path) = app.config.default_database.as_ref() {
		path
	} else {
		default_path = default_new_database_path();
		&default_path
	};
	let filename = Paragraph::new(path.to_string_lossy()).alignment(Alignment::Center).style(Style::default().fg(app.theme.warning));
	frame.render_widget(filename, filename_area);
	if missing && !app.creating_database {
		frame.render_widget(input.block(section), input_area);
	} else {
		super::draw_input(frame, input_area, section, input, &app.theme);
	}
	if let Some(error) = &app.login_error {
		let error_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 3 };

		let error_text = Paragraph::new(error.as_str())
			.alignment(Alignment::Center)
			.wrap(Wrap { trim: true })
			.style(Style::new().fg(app.theme.error));

		frame.render_widget(error_text, error_area);
	} else if app.creating_database {
		let hint_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 2 };
		let hint = Paragraph::new(vec![Line::from(format!("will create {}", path.display())), Line::from("[Enter] Confirm  [Esc] Cancel")])
			.alignment(Alignment::Center)
			.style(Style::new().fg(app.theme.warning));

		frame.render_widget(hint, hint_area);
	} else if missing {
		let hint_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 1 };

		let hint = Paragraph::new("[c] Create database  [^c] Config  [Esc] Quit")
			.alignment(Alignment::Center)
			.style(Style::new().fg(app.theme.warning));

		frame.render_widget(hint, hint_area);
	} else {
		let hint_area = Rect { x: input_area.x, y: input_area.y.saturating_add(input_area.height), width: input_area.width, height: 1 };

		let hint = Paragraph::new("Enter master password  [^c] Config  [Esc] Quit")
			.alignment(Alignment::Center)
			.style(Style::new().fg(app.theme.warning));

		frame.render_widget(hint, hint_area);
	}

	// Only show the cursor when the prompt accepts text.
	if missing && !app.creating_database {
		return;
	}
	let typed_len = input_text.chars().count();

	if inner.width == 0 || inner.height == 0 {
		return;
	}

	let width = inner.width as usize;
	let cursor_offset = if typed_len < width { (width - typed_len) / 2 + typed_len } else { width - 1 };

	let cursor_x = inner.x.saturating_add(cursor_offset as u16);
	frame.set_cursor_position((cursor_x, inner.y));
}
