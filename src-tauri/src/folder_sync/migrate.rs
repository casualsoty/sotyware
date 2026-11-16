//! Moving a sync to another folder (ADR-043), while its engine is stopped.
//!
//! - **Move** (only into an empty folder): every entry of the sync folder
//!   (synced or not: it is the user's folder) goes to the new folder. On
//!   one drive each top-level entry is renamed (instant, nothing copied).
//!   Between drives everything is copied, each file read back and compared
//!   by SHA-256 with what was read, its modification time kept; only when
//!   every file is verified are the originals removed. If anything fails,
//!   the copies made are removed again and the old folder stays the sync
//!   folder, untouched. The sync trash moves along; the marker is written
//!   in the new folder before it is removed from the old one. The synced
//!   state stays valid (paths are relative): a copied file whose stamp
//!   changed is hashed once by the next scan and only recorded, not sent.
//! - **Fresh**: the old folder is left exactly as it is, only its marker
//!   removed (its trash stays); the synced state is forgotten and the new
//!   folder is merged with the bucket like a first sync, so nothing is
//!   deleted on either side because of the switch.
//!
//! The old folder itself is never deleted (after a move it is empty).

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::model::OWN_FOLDER;
use super::scan::{self, Marker, RootState};
use super::{MigrationInfo, MoveMode, Shared, SyncConfig};
use crate::cloud::dashboard::{CommandError, ErrorKind};

/// Tests: copy as between drives even on one.
#[cfg(test)]
pub static FORCE_COPY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn force_copy() -> bool {
    #[cfg(test)]
    {
        FORCE_COPY.load(std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(not(test))]
    {
        false
    }
}

fn local(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(ErrorKind::LocalFile, e.to_string())
}

/// Moves (or switches) the sync `config` to `target`; on success its
/// configuration names `target`.
pub(super) fn run(
    shared: &Shared,
    config: &SyncConfig,
    target: &Path,
    mode: MoveMode,
) -> Result<(), CommandError> {
    let old = PathBuf::from(&config.path);
    let target_text = target
        .to_str()
        .ok_or_else(|| CommandError::new(ErrorKind::InvalidInput, "the path is not valid Unicode"))?
        .to_owned();
    match mode {
        MoveMode::Move => move_files(shared, config, &old, target)?,
        MoveMode::Fresh => {
            shared.with_store(|s| s.forget_synced(&config.id))?;
        }
    }
    let marker = Marker {
        format: 1,
        sync: config.marker.clone(),
        account: config.account.clone(),
        bucket: config.bucket.clone(),
        created_ms: config.created_ms,
    };
    fs::create_dir_all(target).map_err(local)?;
    scan::write_marker(target, &marker).map_err(|e| {
        CommandError::new(
            ErrorKind::LocalPermission,
            format!("cannot mark the new folder as the sync folder: {e}"),
        )
    })?;
    // The old folder is no longer this sync's (only our marker is removed;
    // a folder used again keeps the one just written).
    let same = scan::canonical(&old).ok().as_deref() == Some(target);
    if !same && matches!(scan::read_marker(&old), Ok(Some(m)) if m.sync == config.marker) {
        let _ = scan::remove_marker(&old);
    }
    let mut c = config.clone();
    c.path = target_text;
    let json = serde_json::to_string(&c).map_err(local)?;
    shared.with_store(|s| s.save_config(&c.id, &json))
}

/// One entry of the sync folder to move.
struct Entry {
    name: std::ffi::OsString,
    dir: bool,
}

fn move_files(
    shared: &Shared,
    config: &SyncConfig,
    old: &Path,
    target: &Path,
) -> Result<(), CommandError> {
    if scan::root_state(old, &config.marker) != RootState::Ready {
        return Err(CommandError::new(
            ErrorKind::DestinationUnavailable,
            "the current sync folder is not available, so its files cannot be moved; choose to sync the new folder from scratch instead",
        ));
    }
    fs::create_dir_all(target).map_err(local)?;
    if fs::read_dir(target).map_err(local)?.next().is_some() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "the new folder is not empty any more",
        ));
    }
    let mut entries = Vec::new();
    for e in fs::read_dir(old).map_err(local)? {
        let e = e.map_err(local)?;
        let name = e.file_name();
        let dir = e.file_type().is_ok_and(|t| t.is_dir());
        entries.push(Entry { name, dir });
    }
    let (files_total, bytes_total) = count(old);
    let mut info = MigrationInfo {
        to: target.display().to_string(),
        files_done: 0,
        files_total,
        bytes_done: 0,
        bytes_total,
        copying: false,
    };
    shared.update(&config.id, |s| s.migration = Some(info.clone()));
    // 1. Renames, as long as they work (one drive).
    let mut renamed: Vec<&Entry> = Vec::new();
    let mut rest: Vec<&Entry> = Vec::new();
    for e in &entries {
        if !rest.is_empty() || force_copy() {
            rest.push(e);
            continue;
        }
        let (from, to) = (old.join(&e.name), target.join(&e.name));
        if e.name == OWN_FOLDER {
            // Only the trash moves; the marker and staging stay ours to
            // remove.
            let trash = from.join(scan::TRASH);
            if trash.is_dir() {
                fs::create_dir_all(target.join(OWN_FOLDER)).map_err(local)?;
                if let Err(err) = fs::rename(&trash, to.join(scan::TRASH)) {
                    if err.kind() == io::ErrorKind::CrossesDevices {
                        rest.push(e);
                        continue;
                    }
                    undo_renames(old, target, &renamed);
                    return Err(local(format!("the sync trash could not be moved: {err}")));
                }
            }
            renamed.push(e);
            continue;
        }
        match fs::rename(&from, &to) {
            Ok(()) => renamed.push(e),
            Err(err) if err.kind() == io::ErrorKind::CrossesDevices => rest.push(e),
            Err(err) => {
                undo_renames(old, target, &renamed);
                return Err(local(format!(
                    "{} could not be moved (is a file in it open in another program?): {err}",
                    e.name.to_string_lossy()
                )));
            }
        }
    }
    if rest.is_empty() {
        return Ok(());
    }
    // 2. Copies, verified, between drives.
    info.copying = true;
    shared.update(&config.id, |s| s.migration = Some(info.clone()));
    let mut copied: Vec<PathBuf> = Vec::new();
    let mut progress = |files: u64, bytes: u64| {
        info.files_done += files;
        info.bytes_done += bytes;
        let i = info.clone();
        shared.update(&config.id, |s| s.migration = Some(i));
    };
    for e in &rest {
        let (from, to) = if e.name == OWN_FOLDER {
            (
                old.join(OWN_FOLDER).join(scan::TRASH),
                target.join(OWN_FOLDER).join(scan::TRASH),
            )
        } else {
            (old.join(&e.name), target.join(&e.name))
        };
        if !from.exists() {
            continue;
        }
        let result = if e.dir || e.name == OWN_FOLDER {
            copy_tree(&from, &to, &mut progress)
        } else {
            copy_file(&from, &to).map(|b| progress(1, b))
        };
        copied.push(to.clone());
        if let Err(err) = result {
            for c in copied.iter().rev() {
                let _ = remove_any(c);
            }
            undo_renames(old, target, &renamed);
            return Err(local(format!(
                "{} could not be copied to the new folder; nothing was moved: {err}",
                e.name.to_string_lossy()
            )));
        }
    }
    // Every copy verified: the originals go (the old folder stays).
    for e in &rest {
        let from = if e.name == OWN_FOLDER {
            old.join(OWN_FOLDER).join(scan::TRASH)
        } else {
            old.join(&e.name)
        };
        if let Err(err) = remove_any(&from) {
            eprintln!(
                "sotyware: a moved sync folder entry could not be removed from the old folder: {err}"
            );
        }
    }
    Ok(())
}

/// Puts renamed entries back (a failed move leaves the old folder whole).
fn undo_renames(old: &Path, target: &Path, renamed: &[&Entry]) {
    for e in renamed.iter().rev() {
        let (from, to) = if e.name == OWN_FOLDER {
            (
                target.join(OWN_FOLDER).join(scan::TRASH),
                old.join(OWN_FOLDER).join(scan::TRASH),
            )
        } else {
            (target.join(&e.name), old.join(&e.name))
        };
        if from.exists()
            && let Err(err) = fs::rename(&from, &to)
        {
            eprintln!("sotyware: could not put a moved entry back: {err}");
        }
    }
}

fn count(dir: &Path) -> (u64, u64) {
    let mut out = (0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    if d == dir && e.file_name() == OWN_FOLDER {
                        stack.push(e.path().join(scan::TRASH));
                    } else {
                        stack.push(e.path());
                    }
                }
                Ok(t) if t.is_file() => {
                    out.0 += 1;
                    out.1 += e.metadata().map(|m| m.len()).unwrap_or(0);
                }
                _ => {}
            }
        }
    }
    out
}

/// Copies the folder `from` to `to` (which must not exist), verified.
fn copy_tree(from: &Path, to: &Path, progress: &mut dyn FnMut(u64, u64)) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let kind = e.file_type()?;
        if kind.is_symlink() {
            copy_link(&src, &dst)?;
        } else if kind.is_dir() {
            copy_tree(&src, &dst, progress)?;
        } else if kind.is_file() {
            let b = copy_file(&src, &dst)?;
            progress(1, b);
        } else {
            return Err(io::Error::other(format!(
                "{} is neither a file nor a folder",
                src.display()
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(src)?, dst)
}

/// Windows needs a privilege to create links: a folder holding one cannot
/// be copied (its link would be lost), so the move stops before anything
/// is removed.
#[cfg(not(unix))]
fn copy_link(src: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::other(format!(
        "{} is a link, which cannot be copied to another drive",
        src.display()
    )))
}

/// Copies one file (which must not exist at `to`), syncs it, keeps its
/// modification time, then reads it back: its SHA-256 must be what was
/// read from the original. Returns its size.
fn copy_file(from: &Path, to: &Path) -> io::Result<u64> {
    let mut src = fs::File::open(from)?;
    let modified = src.metadata()?.modified().ok();
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        dst.write_all(&buf[..n])?;
        size += n as u64;
    }
    dst.sync_all()?;
    if let Some(t) = modified {
        dst.set_modified(t)?;
    }
    drop(dst);
    let expected = hasher.finalize();
    let mut check = fs::File::open(to)?;
    let mut h = Sha256::new();
    loop {
        let n = check.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    if h.finalize() != expected {
        return Err(io::Error::other(format!(
            "{}: the copy does not match the original",
            to.display()
        )));
    }
    Ok(size)
}

fn remove_any(p: &Path) -> io::Result<()> {
    match p.symlink_metadata() {
        Ok(m) if m.is_dir() => fs::remove_dir_all(p),
        Ok(_) => fs::remove_file(p),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::test_server::TempDir;

    #[test]
    fn a_verified_copy_keeps_content_and_time() {
        let t = TempDir::new("migrate-copy");
        let src = t.file("a.bin", 3 << 20, 9);
        let when =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        fs::OpenOptions::new()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(when)
            .unwrap();
        let dst = t.path("b.bin");
        assert_eq!(copy_file(&src, &dst).unwrap(), 3 << 20);
        assert_eq!(fs::read(&src).unwrap(), fs::read(&dst).unwrap());
        assert_eq!(fs::metadata(&dst).unwrap().modified().unwrap(), when);
        // Never over an existing file.
        assert!(copy_file(&src, &dst).is_err());
    }

    #[test]
    fn counting_includes_the_trash_but_not_the_rest_of_our_folder() {
        let t = TempDir::new("migrate-count");
        let root = t.path("root");
        fs::create_dir_all(root.join("d")).unwrap();
        fs::write(root.join("d/x"), b"12").unwrap();
        scan::prepare_own_dir(&root).unwrap();
        fs::write(root.join(OWN_FOLDER).join(scan::TRASH).join("t"), b"1").unwrap();
        fs::write(
            root.join(OWN_FOLDER)
                .join(scan::STAGING)
                .join("download-1-1"),
            b"123",
        )
        .unwrap();
        assert_eq!(count(&root), (2, 3));
    }
}
