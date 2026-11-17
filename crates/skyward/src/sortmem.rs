//! The order the file manager shows entries in, kept between starts.

use anyhow::{Context, Result};
use skyward_ui::app::SortKey;
use std::path::Path;

const FILE: &str = "/metadata/skyward/sort";

/// The order kept, if one was.
pub fn read() -> Option<(SortKey, bool, bool)> {
    with_metadata(read_unmounted)
}

/// Read it with /metadata mounted for the moment it takes: the pages are built
/// before the recovery mounts it, and a file read then is a file not found
/// (2026-11-15, the order and the WLAN kept were lost every start).
fn with_metadata<T>(work: impl FnOnce() -> T) -> T {
    let at = std::path::Path::new("/metadata");
    let mounted = skyward_block::mount::is_mount_point(at).unwrap_or(false);
    if !mounted
        && skyward_block::mount::mount_ro(&std::path::Path::new(skyward_block::BY_NAME).join("metadata"), at).is_err()
    {
        return work();
    }
    let done = work();
    if !mounted {
        let _ = skyward_block::mount::umount(at);
    }
    done
}

fn read_unmounted() -> Option<(SortKey, bool, bool)> {
    let text = std::fs::read_to_string(FILE).ok()?;
    let (mut key, mut desc, mut folders) = (SortKey::Name, false, true);
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else { continue };
        match (name.trim(), value.trim()) {
            ("key", "name") => key = SortKey::Name,
            ("key", "size") => key = SortKey::Size,
            ("key", "modified") => key = SortKey::Modified,
            ("key", "type") => key = SortKey::Type,
            ("desc", "1") => desc = true,
            ("desc", "0") => desc = false,
            ("folders", "1") => folders = true,
            ("folders", "0") => folders = false,
            _ => {}
        }
    }
    Some((key, desc, folders))
}

/// Keep `key`, `desc` and `folders` for the starts that follow.
pub fn write(key: SortKey, desc: bool, folders: bool) -> Result<()> {
    let name = match key {
        SortKey::Name => "name",
        SortKey::Size => "size",
        SortKey::Modified => "modified",
        SortKey::Type => "type",
    };
    let body = format!("key={name}\ndesc={}\nfolders={}\n", desc as u8, folders as u8);
    let at = Path::new(FILE);
    // /metadata comes up read only; it is asked for writes for the moment it
    // takes to put the file down, as the kept WLAN's own file is.
    let _ = skyward_block::mount::remount_writable(Path::new("/metadata"), true);
    let written = std::fs::write(at, body).with_context(|| format!("write {}", at.display()));
    let _ = skyward_block::mount::remount_writable(Path::new("/metadata"), false);
    written
}
