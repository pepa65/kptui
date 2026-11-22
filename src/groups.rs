//! The groups (folders) of a database, to browse the Index like a folder tree.
//!
//! The root group is represented by `None` throughout, so a database without
//! groups works exactly as before: everything is in the root.

use std::collections::HashMap;

use keepass::Database;
use keepass::db::{EntryId, GroupId};

/// A group below the root group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupInfo {
	pub id: GroupId,
	/// `None` for a group directly in the root group.
	pub parent: Option<GroupId>,
	pub name: String,
}

/// All groups below the root group, parents before their children.
pub fn groups_of(db: &Database) -> Vec<GroupInfo> {
	let root = db.root().id();
	let mut groups = Vec::new();
	let mut queue = vec![root];
	let mut next = 0;
	while let Some(&id) = queue.get(next) {
		next += 1;
		let Some(group) = db.group(id) else {
			continue;
		};
		for child in group.group_ids() {
			if let Some(child_group) = db.group(child) {
				groups.push(GroupInfo { id: child, parent: (id != root).then_some(id), name: child_group.name.clone() });
				queue.push(child);
			}
		}
	}
	groups
}

/// The group of every entry that isn't in the root group.
pub fn entry_groups(db: &Database) -> HashMap<EntryId, GroupId> {
	groups_of(db)
		.into_iter()
		.filter_map(|group| db.group(group.id).map(|found| found.entry_ids().map(move |entry| (entry, group.id)).collect::<Vec<_>>()))
		.flatten()
		.collect()
}

/// The path of a group, like `/Internet/Email`, or `/` for the root group.
pub fn path(groups: &[GroupInfo], id: Option<GroupId>) -> String {
	let mut names = Vec::new();
	let mut current = id;
	// The depth can't exceed the number of groups, which also stops a loop.
	while let Some(group) = current.and_then(|id| groups.iter().find(|group| group.id == id))
		&& names.len() < groups.len()
	{
		names.push(group.name.as_str());
		current = group.parent;
	}
	names.reverse();
	format!("/{}", names.join("/"))
}

/// The subgroups of a group (`None`: the root), as indices into `groups`, sorted by name.
pub fn children(groups: &[GroupInfo], parent: Option<GroupId>) -> Vec<usize> {
	let mut children: Vec<usize> = (0..groups.len()).filter(|&index| groups[index].parent == parent).collect();
	children.sort_by_cached_key(|&index| groups[index].name.to_lowercase());
	children
}

#[cfg(test)]
mod tests {
	use super::{children, entry_groups, groups_of, path};
	use keepass::Database;
	use keepass::db::fields;

	/// Root with entry "Router" and groups "internet" (with "Email", holding
	/// "Gmail") and "Bank".
	fn database() -> Database {
		let mut db = Database::new();
		let mut root = db.root_mut();
		root.add_entry().set_unprotected(fields::TITLE, "Router");
		let internet = {
			let mut group = root.add_group();
			group.name = "internet".to_string();
			group.id()
		};
		root.add_group().name = "Bank".to_string();
		let mut internet = db.group_mut(internet).expect("group should exist");
		let mut email = internet.add_group();
		email.name = "Email".to_string();
		email.add_entry().set_unprotected(fields::TITLE, "Gmail");
		db
	}

	#[test]
	fn lists_groups_with_their_parents() {
		let db = database();
		let groups = groups_of(&db);
		let names: Vec<&str> = groups.iter().map(|group| group.name.as_str()).collect();
		assert_eq!(names, ["internet", "Bank", "Email"]);
		assert_eq!(groups[0].parent, None);
		assert_eq!(groups[2].parent, Some(groups[0].id));
	}

	#[test]
	fn builds_paths() {
		let groups = groups_of(&database());
		assert_eq!(path(&groups, None), "/");
		assert_eq!(path(&groups, Some(groups[0].id)), "/internet");
		assert_eq!(path(&groups, Some(groups[2].id)), "/internet/Email");
	}

	#[test]
	fn sorts_subgroups_by_name() {
		let groups = groups_of(&database());
		let names = |parent| children(&groups, parent).into_iter().map(|index| groups[index].name.clone()).collect::<Vec<_>>();
		assert_eq!(names(None), ["Bank", "internet"]);
		assert_eq!(names(Some(groups[0].id)), ["Email"]);
		assert!(names(Some(groups[2].id)).is_empty());
	}

	#[test]
	fn finds_the_group_of_each_entry() {
		let db = database();
		let groups = groups_of(&db);
		let owners = entry_groups(&db);
		for entry in db.iter_all_entries() {
			let expected = if entry.get_title() == Some("Gmail") { Some(groups[2].id) } else { None };
			assert_eq!(owners.get(&entry.id()).copied(), expected, "{:?}", entry.get_title());
		}
	}
}
