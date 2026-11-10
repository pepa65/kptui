use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};
use zeroize::Zeroizing;

const PRIMARY: AtomEnum = AtomEnum::PRIMARY;

pub struct Primary {
	root: Window,
	utf8: Atom,
	text: Atom,
	targets: Atom,
	timestamp: Atom,
	owned: Option<OwnedSelection>,
}

struct OwnedSelection {
	window: Window,
	time: Timestamp,
	bytes: Zeroizing<Vec<u8>>,
}

impl Primary {
	pub fn new(conn: &impl Connection, root: Window) -> Result<Self, ReplyError> {
		let utf8 = conn.intern_atom(false, b"UTF8_STRING")?;
		let text = conn.intern_atom(false, b"TEXT")?;
		let targets = conn.intern_atom(false, b"TARGETS")?;
		let timestamp = conn.intern_atom(false, b"TIMESTAMP")?;
		Ok(Self {
			root,
			utf8: utf8.reply()?.atom,
			text: text.reply()?.atom,
			targets: targets.reply()?.atom,
			timestamp: timestamp.reply()?.atom,
			owned: None,
		})
	}

	/// Keep the selection in a wipeable allocation and serve it only on demand.
	pub fn publish(&mut self, conn: &impl Connection, text: &str, time: Timestamp) -> anyhow::Result<bool> {
		self.clear(conn)?;
		if text.is_empty() || text.len() > conn.maximum_request_bytes().saturating_sub(32) {
			return Ok(false);
		}

		// Destroying this window releases only our ownership, even if another
		// client acquires PRIMARY while we are clearing our local selection.
		let window = conn.generate_id()?;
		conn.create_window(0, window, self.root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?.check()?;
		conn.set_selection_owner(window, PRIMARY.into(), time)?.check()?;
		if conn.get_selection_owner(PRIMARY.into())?.reply()?.owner != window {
			conn.destroy_window(window)?;
			return Ok(false);
		}

		self.owned = Some(OwnedSelection { window, time, bytes: Zeroizing::new(text.as_bytes().to_vec()) });
		Ok(true)
	}

	pub fn clear(&mut self, conn: &impl Connection) -> Result<(), ReplyError> {
		if let Some(selection) = self.owned.take() {
			peer_request_succeeded(conn.destroy_window(selection.window)?.check())?;
		}
		Ok(())
	}

	pub fn lost(&mut self, conn: &impl Connection, event: SelectionClearEvent) -> Result<bool, ReplyError> {
		if event.selection == Atom::from(PRIMARY) && self.owned.as_ref().is_some_and(|selection| selection.window == event.owner) {
			self.clear(conn)?;
			return Ok(true);
		}
		Ok(false)
	}

	pub fn respond(&self, conn: &impl Connection, request: SelectionRequestEvent) -> Result<(), ReplyError> {
		let property = if request.property == NONE { request.target } else { request.property };
		let mut accepted = false;
		if let Some(selection) = &self.owned
			&& request.owner == selection.window
			&& request.selection == Atom::from(PRIMARY)
			&& valid_request_time(request.time, selection.time)
		{
			let ascii = selection.bytes.is_ascii();
			let result = if request.target == self.targets {
				let targets = [self.targets, self.timestamp, self.utf8, self.text, AtomEnum::STRING.into()];
				let supported = if ascii { &targets[..] } else { &targets[..4] };
				Some(conn.change_property32(PropMode::REPLACE, request.requestor, property, AtomEnum::ATOM, supported)?.check())
			} else if request.target == self.timestamp {
				Some(conn.change_property32(PropMode::REPLACE, request.requestor, property, AtomEnum::INTEGER, &[selection.time])?.check())
			} else if request.target == self.utf8 || request.target == self.text {
				Some(conn.change_property8(PropMode::REPLACE, request.requestor, property, self.utf8, &selection.bytes)?.check())
			} else if ascii && request.target == Atom::from(AtomEnum::STRING) {
				Some(conn.change_property8(PropMode::REPLACE, request.requestor, property, AtomEnum::STRING, &selection.bytes)?.check())
			} else {
				None
			};

			if let Some(result) = result {
				accepted = peer_request_succeeded(result)?;
			}
		}

		let reply = SelectionNotifyEvent {
			response_type: SELECTION_NOTIFY_EVENT,
			sequence: 0,
			time: request.time,
			requestor: request.requestor,
			selection: request.selection,
			target: request.target,
			property: if accepted { property } else { NONE },
		};
		peer_request_succeeded(conn.send_event(false, request.requestor, EventMask::NO_EVENT, reply)?.check())?;
		Ok(())
	}
}

fn valid_request_time(request: Timestamp, acquired: Timestamp) -> bool {
	request == CURRENT_TIME || request.wrapping_sub(acquired) < (1 << 31)
}

fn peer_request_succeeded(result: Result<(), ReplyError>) -> Result<bool, ReplyError> {
	match result {
		Ok(()) => Ok(true),
		// The requestor may have closed its window or supplied an invalid atom.
		Err(ReplyError::X11Error(_)) => Ok(false),
		Err(err) => Err(err),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn stale_requests_are_rejected_across_timestamp_wrap() {
		assert!(valid_request_time(CURRENT_TIME, 100));
		assert!(!valid_request_time(99, 100));
		assert!(valid_request_time(100, 100));
		assert!(valid_request_time(5, u32::MAX - 5));
		assert!(!valid_request_time(u32::MAX - 5, 5));
	}
}
