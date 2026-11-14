//! Pure rules of the file-management operations (ADR-026): what a new name
//! may be, which keys belong to a folder, where an object goes when its
//! item is copied or moved, and the next free name for a conflict. No
//! request is sent here; everything is unit-tested.

use std::collections::HashSet;

/// Longest object key S3 allows, in bytes.
pub const MAX_KEY_BYTES: usize = 1024;

/// Why a name typed by the user cannot be used.
pub fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("a name cannot be empty");
    }
    if name.trim().is_empty() {
        return Some("a name cannot be only spaces");
    }
    if name.contains('/') {
        return Some("a name cannot contain '/' (it separates folders in object keys)");
    }
    if name.chars().any(char::is_control) {
        return Some("a name cannot contain control characters");
    }
    if name == "." || name == ".." {
        return Some("'.' and '..' cannot be used as names");
    }
    None
}

/// The last segment of a file key (the whole key if it has no `/`).
pub fn file_name(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// The name of a folder prefix (`a/b/` → `b`; may be empty for `a//`).
pub fn folder_name(prefix: &str) -> &str {
    let trimmed = prefix.strip_suffix('/').unwrap_or(prefix);
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// The folder a key or a folder prefix lies in: `a/b/c.txt` → `a/b/`,
/// `a/b/` → `a/`, `c.txt` → `` (the root).
pub fn parent_of(path: &str) -> &str {
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    match trimmed.rfind('/') {
        Some(i) => &path[..=i],
        None => "",
    }
}

/// Whether `key` is inside the folder `prefix`. `prefix` must end in `/`,
/// so `photos/` never contains `photos-backup/x`.
pub fn in_folder(prefix: &str, key: &str) -> bool {
    prefix.ends_with('/') && key.starts_with(prefix)
}

/// `key` with the folder `from` (a prefix ending in `/`) replaced by `to`.
pub fn relocate(key: &str, from: &str, to: &str) -> Option<String> {
    key.strip_prefix(from).map(|rest| format!("{to}{rest}"))
}

/// `name (n).ext`: before the last `.` unless the name starts with its only
/// dot (`.env` → `.env (1)`), or has no dot.
pub fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

/// Whether moving or copying the folder `source` into the folder
/// `destination` would put it inside itself.
pub fn into_itself(source: &str, destination: &str) -> bool {
    destination.starts_with(source)
}

/// Drops selected keys and folders that lie inside another selected
/// folder (they are part of it), and duplicates. Order is kept.
pub fn without_nested(files: Vec<String>, folders: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut seen = HashSet::new();
    let outer: Vec<String> = folders
        .iter()
        .filter(|f| !folders.iter().any(|o| o != *f && in_folder(o, f)))
        .filter(|f| seen.insert((*f).clone()))
        .cloned()
        .collect();
    let files = files
        .into_iter()
        .filter(|k| !outer.iter().any(|o| in_folder(o, k)))
        .filter(|k| seen.insert(k.clone()))
        .collect();
    (files, outer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_cannot_be_used() {
        for bad in ["", "  ", "a/b", "x\u{0}", "tab\there", ".", ".."] {
            assert!(name_problem(bad).is_some(), "{bad:?}");
        }
        for good in ["a", "größe +test.bin", "..a", "a..", " lead", "#%?&", "家"] {
            assert_eq!(name_problem(good), None, "{good:?}");
        }
    }

    #[test]
    fn names_and_parents_of_keys_and_prefixes() {
        assert_eq!(file_name("a/b/c.txt"), "c.txt");
        assert_eq!(file_name("c.txt"), "c.txt");
        assert_eq!(folder_name("a/b/"), "b");
        assert_eq!(folder_name("a//"), "");
        assert_eq!(parent_of("a/b/c.txt"), "a/b/");
        assert_eq!(parent_of("a/b/"), "a/");
        assert_eq!(parent_of("a/"), "");
        assert_eq!(parent_of("c.txt"), "");
        assert_eq!(parent_of("a//"), "a/");
    }

    #[test]
    fn a_folder_never_contains_a_neighbour_with_the_same_start() {
        assert!(in_folder("photos/", "photos/a.jpg"));
        assert!(in_folder("photos/", "photos/"));
        assert!(!in_folder("photos/", "photos-backup/a.jpg"));
        assert!(!in_folder("photos/", "photos"));
        assert!(
            !in_folder("photos", "photos-backup/a.jpg"),
            "a prefix without '/' is no folder"
        );
    }

    #[test]
    fn relocating_keeps_the_rest_of_the_key_exactly() {
        assert_eq!(
            relocate("a/b/x//y z", "a/b/", "c/b/").as_deref(),
            Some("c/b/x//y z")
        );
        assert_eq!(relocate("a/b/", "a/b/", "c/").as_deref(), Some("c/"));
        assert_eq!(relocate("z/x", "a/", "c/"), None);
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered("report.pdf", 1), "report (1).pdf");
        assert_eq!(numbered("archive.tar.gz", 2), "archive.tar (2).gz");
        assert_eq!(numbered(".env", 1), ".env (1)");
        assert_eq!(numbered("README", 3), "README (3)");
    }

    #[test]
    fn a_folder_cannot_go_into_itself() {
        assert!(into_itself("a/", "a/"));
        assert!(into_itself("a/", "a/b/"));
        assert!(!into_itself("a/", "ab/"));
        assert!(!into_itself("a/b/", "a/"));
    }

    #[test]
    fn nested_selections_are_dropped() {
        let (files, folders) = without_nested(
            vec![
                "a/x.txt".into(),
                "b.txt".into(),
                "b.txt".into(),
                "a-other/y".into(),
            ],
            vec!["a/".into(), "a/sub/".into(), "c/".into()],
        );
        assert_eq!(files, vec!["b.txt".to_owned(), "a-other/y".to_owned()]);
        assert_eq!(folders, vec!["a/".to_owned(), "c/".to_owned()]);
    }
}
