use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::ffi::{c_int, c_uint, c_ulong, c_void};
use std::ptr::{NonNull, null};
use x11rb::protocol::xproto::KeyButMask;
use xkeysym::Keysym;

unsafe extern "C" {
	fn XOpenDisplay(name: *const std::ffi::c_char) -> *mut c_void;
	fn XCloseDisplay(display: *mut c_void) -> c_int;
	fn XkbLookupKeySym(display: *mut c_void, keycode: u8, state: c_uint, unconsumed: *mut c_uint, keysym: *mut c_ulong) -> c_int;
}

pub struct Keyboard(NonNull<c_void>);

impl Keyboard {
	pub fn open() -> Result<Self, &'static str> {
		// Use the same DISPLAY as x11rb. Xlib owns this keyboard-map connection.
		let display = unsafe { XOpenDisplay(null()) };
		NonNull::new(display).map(Self).ok_or("could not open XKB keyboard map")
	}

	pub fn symbol(&self, keycode: u8, state: KeyButMask) -> Keysym {
		let mut symbol = 0;
		let mut unconsumed = 0;
		// Pass the entire event state, including XKB's group bits (13 and 14).
		unsafe { XkbLookupKeySym(self.0.as_ptr(), keycode, state.into(), &mut unconsumed, &mut symbol) };
		Keysym::new(symbol as u32)
	}
}

impl Drop for Keyboard {
	fn drop(&mut self) {
		unsafe { XCloseDisplay(self.0.as_ptr()) };
	}
}

impl Keyboard {
	pub fn key_event(&self, code: u8, state: KeyButMask) -> Option<KeyEvent> {
		let mut modifiers = KeyModifiers::NONE;
		modifiers.set(KeyModifiers::SHIFT, state.contains(KeyButMask::SHIFT));
		modifiers.set(KeyModifiers::CONTROL, state.contains(KeyButMask::CONTROL));
		modifiers.set(KeyModifiers::ALT, state.contains(KeyButMask::MOD1));
		modifiers.set(KeyModifiers::SUPER, state.contains(KeyButMask::MOD4));

		let mut symbol = self.symbol(code, state);
		if modifiers.contains(KeyModifiers::CONTROL) && symbol.key_char().is_some_and(|c| !c.is_ascii()) {
			// Keep Ctrl shortcuts reachable while a non-Latin group is active.
			symbol = self.symbol(code, KeyButMask::from(u16::from(state) & !(3 << 13)));
		}

		translate(symbol, modifiers)
	}
}

pub(super) fn translate(symbol: Keysym, modifiers: KeyModifiers) -> Option<KeyEvent> {
	use xkeysym::key;
	let key = match symbol.raw() {
		key::Escape => KeyCode::Esc,
		key::Return | key::KP_Enter => KeyCode::Enter,
		key::BackSpace => KeyCode::Backspace,
		key::Delete | key::KP_Delete => KeyCode::Delete,
		key::Tab => KeyCode::Tab,
		key::ISO_Left_Tab => KeyCode::BackTab,
		key::Up | key::KP_Up => KeyCode::Up,
		key::Down | key::KP_Down => KeyCode::Down,
		key::Left | key::KP_Left => KeyCode::Left,
		key::Right | key::KP_Right => KeyCode::Right,
		key::Page_Up | key::KP_Page_Up => KeyCode::PageUp,
		key::Page_Down | key::KP_Page_Down => KeyCode::PageDown,
		key::Home | key::KP_Home => KeyCode::Home,
		key::End | key::KP_End => KeyCode::End,
		_ => KeyCode::Char(symbol.key_char().filter(|c| !c.is_control())?),
	};
	Some(KeyEvent::new(key, modifiers))
}
