pub mod command;
pub mod edit;
pub mod fieldedit;
pub mod fieldview;
pub mod index;
pub mod login;
pub mod settings;

use crossterm::event::KeyCode;
use zeroize::Zeroize;

pub const SECRET_CAP: usize = 1024;

pub fn normalize_shortcut(key: KeyCode) -> KeyCode {
	match key {
		KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
		other => other,
	}
}

pub fn secret_push(secret: &mut String, c: char) -> bool {
	if secret.len() + c.len_utf8() > secret.capacity() {
		return false;
	}

	secret.push(c);
	true
}

pub fn secret_pop(secret: &mut String) {
	if let Some((index, _)) = secret.char_indices().next_back() {
		secret[index..].zeroize();
		secret.truncate(index);
	}
}
