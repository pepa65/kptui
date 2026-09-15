use std::time::Instant;

use arboard::Clipboard;

use crate::app::App;

pub struct ClipboardTimer {
	pub label: String,
	pub expected: String,
	pub clear_at: Instant,
}

fn copy_and_report(app: &mut App, label: &str, text: &str) {
	let result = match app.clipboard.as_mut() {
		Some(clipboard) => clipboard.set_text(text),
		None => match Clipboard::new() {
			Ok(mut clipboard) => {
				let result = clipboard.set_text(text);
				app.clipboard = Some(clipboard);
				result
			}
			Err(err) => Err(err),
		},
	};

	match result {
		Ok(()) => {
			app.clipboard_timer =
				Some(ClipboardTimer { label: label.to_string(), expected: text.to_string(), clear_at: Instant::now() + app.config.clipboard_timeout });
			app.status = None;
		}
		Err(err) => {
			app.clipboard_timer = None;
			app.status = Some(format!("Failed to copy {label}: {err}"));
		}
	}
}

pub fn maybe_clear_clipboard(app: &mut App) {
	let Some(timer) = app.clipboard_timer.as_ref() else {
		return;
	};

	if Instant::now() < timer.clear_at {
		return;
	}

	let expected = std::mem::take(&mut app.clipboard_timer).unwrap().expected;

	if let Some(clipboard) = app.clipboard.as_mut() {
		let holds_ours = matches!(clipboard.get_text(), Ok(current) if current == expected);

		if holds_ours {
			if let Err(err) = clipboard.clear() {
				app.status = Some(format!("Couldn't clear clipboard: {err}"));
				return;
			}
		}
	}

	app.status = Some("Clipboard cleared".to_string());
}

pub fn cp_user(app: &mut App) {
	if let Some(entry) = app.selected_entry() {
		let user = entry.user.clone();
		copy_and_report(app, "user", &user);
	}
}

pub fn cp_password(app: &mut App) {
	if let Some(entry) = app.selected_entry() {
		let password = entry.password.clone();
		copy_and_report(app, "password", &password);
	}
}

pub fn cp_totp(app: &mut App) {
	let Some(entry) = app.selected_entry() else {
		return;
	};

	match crate::db::current_totp_code(&entry.totp) {
		Some(totp) => copy_and_report(app, "TOTP code", &totp.code),
		None => {
			app.clipboard_timer = None;
			app.status = Some("No valid TOTP configured for this entry".to_string());
		}
	}
}

pub fn cp_url(app: &mut App) {
	if let Some(entry) = app.selected_entry() {
		let url = entry.url.clone();
		copy_and_report(app, "URL", &url);
	}
}
