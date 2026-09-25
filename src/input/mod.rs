pub mod command;
pub mod edit;
pub mod fieldedit;
pub mod fieldview;
pub mod index;
pub mod login;
pub mod settings;

use crossterm::event::KeyCode;
use zeroize::Zeroize;

pub fn normalize_shortcut(key: KeyCode) -> KeyCode {
	match key {
		KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
		other => other,
	}
}

pub fn secret_pop(secret: &mut String) {
	if let Some((index, _)) = secret.char_indices().next_back() {
		secret[index..].zeroize();
		secret.truncate(index);
	}
}
