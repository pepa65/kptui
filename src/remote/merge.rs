//! Three-way merge, for a database that changed both here and on the server.
//!
//! `base` is the last version both sides had in common. Starting from the
//! server copy, the changes made here since `base` are applied on top of it:
//! - Entries added on either side are kept.
//! - An entry changed on one side only gets that side's version.
//! - An entry changed on both sides is merged field by field. When both sides
//!   changed the same field differently, the most recently modified version
//!   wins and the other one is added as a separate entry, with " (conflict)"
//!   after its title, so nothing gets lost.
//! - An entry deleted on one side is deleted, unless the other side changed
//!   it: a change wins over a deletion.
//!
//! An empty field counts the same as a missing one.

use std::collections::{HashMap, HashSet};

use keepass::Database;
use keepass::db::{Entry, EntryId, GroupId, Times, Value, fields};

/// What the merge had to resolve.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
	/// Entries changed differently on both sides, kept twice.
	pub conflicts: usize,
	/// Entries deleted on one side but kept, because they changed on the other.
	pub kept: usize,
}

pub fn merge(base: Option<&Database>, local: &Database, server: &Database) -> (Database, Report) {
	let mut merged = server.clone();
	let mut report = Report::default();
	let local_parents = parents(local);
	let server_parents = parents(server);

	let mut ids: Vec<EntryId> = local.iter_all_entries().map(|entry| entry.id()).collect();
	ids.extend(server.iter_all_entries().map(|entry| entry.id()));
	if let Some(base) = base {
		ids.extend(base.iter_all_entries().map(|entry| entry.id()));
	}
	let mut seen = HashSet::new();
	ids.retain(|id| seen.insert(*id));
	// A stable order, so conflict copies always end up the same way.
	ids.sort_by_key(EntryId::uuid);

	for id in ids {
		let base_entry = base.and_then(|base| base.entry(id));
		let local_entry = local.entry(id);
		let server_entry = server.entry(id);
		match (base_entry.as_deref(), local_entry.as_deref(), server_entry.as_deref()) {
			(_, None, None) | (None, None, Some(_)) => {}
			// Added here.
			(None, Some(local), None) => add(&mut merged, Some(id), local.fields.clone(), local.times.clone(), local_parents.get(&id)),
			// Deleted here: only when the server didn't change it.
			(Some(base), None, Some(server)) => {
				if same_entry(base, server) {
					remove(&mut merged, id);
				} else {
					report.kept += 1;
				}
			}
			// Deleted on the server: kept when it was changed here.
			(Some(base), Some(local), None) => {
				if !same_entry(base, local) {
					add(&mut merged, Some(id), local.fields.clone(), local.times.clone(), local_parents.get(&id));
					report.kept += 1;
				}
			}
			(base, Some(local), Some(server)) => {
				if same_entry(local, server) || base.is_some_and(|base| same_entry(base, local)) {
					// Unchanged here: the server version is already in place.
				} else if base.is_some_and(|base| same_entry(base, server)) {
					replace(&mut merged, id, local.fields.clone(), local.times.clone());
				} else if merge_fields(&mut merged, id, base, local, server, server_parents.get(&id)) {
					report.conflicts += 1;
				}
			}
		}
	}

	if base.is_none_or(|base| base.config.kdf_config != local.config.kdf_config) {
		merged.config.kdf_config = local.config.kdf_config.clone();
	}
	(merged, report)
}

/// Merges an entry changed on both sides. Returns whether a field conflicted.
fn merge_fields(merged: &mut Database, id: EntryId, base: Option<&Entry>, local: &Entry, server: &Entry, parent: Option<&GroupId>) -> bool {
	let local_wins = local.times.last_modification >= server.times.last_modification;
	let mut keys: HashSet<&String> = local.fields.keys().chain(server.fields.keys()).collect();
	if let Some(base) = base {
		keys.extend(base.fields.keys());
	}

	let mut conflict = false;
	let mut result = HashMap::new();
	for key in keys {
		let base_value = base.map_or("", |base| text(base.fields.get(key)));
		let (local_value, server_value) = (local.fields.get(key), server.fields.get(key));
		let pick = if text(local_value) == text(server_value) || text(local_value) == base_value {
			server_value
		} else if text(server_value) == base_value {
			local_value
		} else {
			conflict = true;
			if local_wins { local_value } else { server_value }
		};
		if let Some(value) = pick {
			result.insert(key.clone(), value.clone());
		}
	}
	replace(merged, id, result, Times::new());

	if conflict {
		let loser = if local_wins { server } else { local };
		let mut copy = loser.fields.clone();
		let title = format!("{} (conflict)", loser.get_title().unwrap_or_default());
		copy.insert(fields::TITLE.to_string(), Value::unprotected(title));
		add(merged, None, copy, loser.times.clone(), parent);
	}
	conflict
}

fn text(value: Option<&Value<String>>) -> &str {
	value.map_or("", |value| value.get().as_str())
}

fn same_entry(a: &Entry, b: &Entry) -> bool {
	a.fields.keys().chain(b.fields.keys()).all(|key| text(a.fields.get(key)) == text(b.fields.get(key)))
}

/// The group of every entry.
fn parents(db: &Database) -> HashMap<EntryId, GroupId> {
	let mut parents = HashMap::new();
	let mut groups = vec![db.root().id()];
	while let Some(id) = groups.pop() {
		let Some(group) = db.group(id) else {
			continue;
		};
		parents.extend(group.entry_ids().map(|entry| (entry, id)));
		groups.extend(group.group_ids());
	}
	parents
}

/// Adds an entry to `parent`, or to the root group if that doesn't exist.
fn add(db: &mut Database, id: Option<EntryId>, fields: HashMap<String, Value<String>>, times: Times, parent: Option<&GroupId>) {
	let group = parent.copied().filter(|&group| db.group(group).is_some()).unwrap_or_else(|| db.root().id());
	let Some(mut group) = db.group_mut(group) else {
		return;
	};
	let entry = match id {
		Some(id) => group.add_entry_with_id(id).ok(),
		None => Some(group.add_entry()),
	};
	if let Some(mut entry) = entry {
		entry.fields = fields;
		entry.times = times;
	}
}

fn replace(db: &mut Database, id: EntryId, fields: HashMap<String, Value<String>>, times: Times) {
	if let Some(mut entry) = db.entry_mut(id) {
		entry.fields = fields;
		entry.times = times;
	}
}

fn remove(db: &mut Database, id: EntryId) {
	if let Some(entry) = db.entry_mut(id) {
		entry.remove();
	}
}

#[cfg(test)]
mod tests {
	use super::{Report, merge};
	use keepass::Database;
	use keepass::db::{EntryId, Times, fields};

	fn add(db: &mut Database, title: &str, password: &str) -> EntryId {
		let mut root = db.root_mut();
		let mut entry = root.add_entry();
		entry.set_unprotected(fields::TITLE, title);
		entry.set_protected(fields::PASSWORD, password);
		entry.id()
	}

	fn set(db: &mut Database, id: EntryId, field: &str, value: &str) {
		let mut entry = db.entry_mut(id).expect("entry should exist");
		entry.set_protected(field, value);
		entry.times.last_modification = Some(Times::now());
	}

	fn get(db: &Database, id: EntryId, field: &str) -> Option<String> {
		db.entry(id).and_then(|entry| entry.get(field).map(str::to_string))
	}

	fn titles(db: &Database) -> Vec<String> {
		let mut titles: Vec<String> = db.iter_all_entries().map(|entry| entry.get_title().unwrap_or_default().to_string()).collect();
		titles.sort();
		titles
	}

	/// A base with one entry, and two copies of it for here and the server.
	fn setup() -> (Database, Database, Database, EntryId) {
		let mut base = Database::new();
		let id = add(&mut base, "Mail", "old");
		(base.clone(), base.clone(), base, id)
	}

	#[test]
	fn takes_the_changes_of_either_side() {
		let (base, mut local, mut server, id) = setup();
		set(&mut local, id, fields::PASSWORD, "new here");
		let (merged, report) = merge(Some(&base), &local, &server);
		assert_eq!(get(&merged, id, fields::PASSWORD).as_deref(), Some("new here"));
		assert_eq!(report, Report::default());

		set(&mut server, id, fields::PASSWORD, "new there");
		let (merged, _) = merge(Some(&base), &base, &server);
		assert_eq!(get(&merged, id, fields::PASSWORD).as_deref(), Some("new there"));
	}

	#[test]
	fn merges_different_fields_changed_on_both_sides() {
		let (base, mut local, mut server, id) = setup();
		set(&mut local, id, fields::PASSWORD, "new password");
		set(&mut server, id, fields::NOTES, "new notes");
		let (merged, report) = merge(Some(&base), &local, &server);
		assert_eq!(get(&merged, id, fields::PASSWORD).as_deref(), Some("new password"));
		assert_eq!(get(&merged, id, fields::NOTES).as_deref(), Some("new notes"));
		assert_eq!(report.conflicts, 0);
		assert_eq!(merged.iter_all_entries().count(), 1);
	}

	#[test]
	fn keeps_both_versions_of_a_conflicting_field() {
		let (base, mut local, mut server, id) = setup();
		set(&mut server, id, fields::PASSWORD, "older");
		set(&mut local, id, fields::PASSWORD, "newer");
		server.entry_mut(id).expect("entry should exist").times.last_modification = None;
		let (merged, report) = merge(Some(&base), &local, &server);
		assert_eq!(report.conflicts, 1);
		assert_eq!(get(&merged, id, fields::PASSWORD).as_deref(), Some("newer"));
		let copy = merged.iter_all_entries().find(|entry| entry.id() != id).expect("the other version should be kept");
		assert_eq!(copy.get_title(), Some("Mail (conflict)"));
		assert_eq!(copy.get_password(), Some("older"));
	}

	#[test]
	fn a_change_wins_over_a_deletion() {
		let (base, mut local, mut server, id) = setup();
		local.entry_mut(id).expect("entry should exist").remove();
		let (merged, _) = merge(Some(&base), &local, &server);
		assert!(merged.entry(id).is_none(), "deleted here and unchanged on the server");

		set(&mut server, id, fields::PASSWORD, "changed there");
		let (merged, report) = merge(Some(&base), &local, &server);
		assert_eq!(get(&merged, id, fields::PASSWORD).as_deref(), Some("changed there"));
		assert_eq!(report.kept, 1);

		let (base, mut local, mut server, id) = setup();
		server.entry_mut(id).expect("entry should exist").remove();
		let (merged, _) = merge(Some(&base), &local, &server);
		assert!(merged.entry(id).is_none(), "deleted on the server and unchanged here");

		set(&mut local, id, fields::PASSWORD, "changed here");
		let (merged, report) = merge(Some(&base), &local, &server);
		assert_eq!(get(&merged, id, fields::PASSWORD).as_deref(), Some("changed here"));
		assert_eq!(report.kept, 1);
	}

	#[test]
	fn keeps_entries_added_on_both_sides() {
		let (base, mut local, mut server, _) = setup();
		add(&mut local, "Bank", "1");
		add(&mut server, "Shop", "2");
		let (merged, report) = merge(Some(&base), &local, &server);
		assert_eq!(titles(&merged), ["Bank", "Mail", "Shop"]);
		assert_eq!(report.conflicts, 0);

		let (merged, _) = merge(None, &local, &server);
		assert_eq!(titles(&merged), ["Bank", "Mail", "Shop"]);
	}

	#[test]
	fn entries_added_here_keep_their_group() {
		let (mut base, _, _, _) = setup();
		let group = base.root_mut().add_group().id();
		let (mut local, mut server) = (base.clone(), base.clone());
		let id = local.group_mut(group).expect("group should exist").add_entry().id();
		server.root_mut().add_entry().set_unprotected(fields::TITLE, "Elsewhere");
		let (merged, _) = merge(Some(&base), &local, &server);
		assert!(merged.group(group).expect("group should exist").entry(id).is_some());
	}

	#[test]
	fn an_empty_field_is_the_same_as_a_missing_one() {
		let (base, mut local, mut server, id) = setup();
		local.entry_mut(id).expect("entry should exist").set_protected(fields::OTP, "");
		set(&mut server, id, fields::NOTES, "server notes");
		let (merged, report) = merge(Some(&base), &local, &server);
		assert_eq!(report.conflicts, 0);
		assert_eq!(get(&merged, id, fields::NOTES).as_deref(), Some("server notes"));
	}
}
