//! The screen lock a user chose to have kept, so that the data partition
//! opens as the recovery starts rather than waiting for it to be typed.
//!
//! It is kept where the recovery keeps its own settings, readable by root
//! alone. A factory reset formats that partition, and so does the way out of
//! the decrypt page that forgets it.

use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The partition the file is on. The recovery mounts it read-only, as the
/// phone's own recovery does, so it is made writable for as long as this
/// takes and then read-only again.
const METADATA: &str = "/metadata";

/// Where it is kept: beside the recovery's own settings.
pub const FILE: &str = "/metadata/skyward/credential";

/// Where a build before this one kept it, on the persist partition, which the
/// recovery mounts as it starts: a credential kept there still opens the
/// files, and is moved to [`FILE`] once it does.
pub const OLD_FILE: &str = "/mnt/vendor/persist/skyward/unlock";

/// The screen lock kept from before, where one was kept.
///
/// /metadata is not mounted yet when the pages are built -- the recovery mounts
/// it read-only for the moment it takes to read its settings, and takes it away
/// again -- so it is mounted here the same way. Read without that, the file is
/// never found and the partition waits to be typed every start (2026-11-15).
pub fn read() -> Option<Vec<u8>> {
    let at = Path::new(METADATA);
    let mounted = skyward_block::mount::is_mount_point(at).unwrap_or(false);
    if !mounted && skyward_block::mount::mount_ro(&Path::new(skyward_block::BY_NAME).join("metadata"), at).is_err() {
        return None;
    }
    let kept = std::fs::read(FILE).ok().filter(|kept| !kept.is_empty());
    if !mounted {
        let _ = skyward_block::mount::umount(at);
    }
    // The copy an earlier build kept on persist, where this one has none.
    kept.or_else(|| std::fs::read(OLD_FILE).ok().filter(|kept| !kept.is_empty()))
}

/// Do `work` with the partition writable, and read-only again afterwards.
fn writable<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    let at = Path::new(METADATA);
    let mounted = skyward_block::mount::is_mount_point(at)?;
    if mounted {
        skyward_block::mount::remount_writable(at, true).context("make /metadata writable")?;
    }
    let done = work();
    if mounted {
        let _ = skyward_block::mount::remount_writable(at, false);
    }
    done
}

/// Keep `credential` for the starts that follow.
pub fn write(credential: &[u8]) -> Result<()> {
    writable(|| {
    let at = Path::new(FILE);
    if let Some(dir) = at.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("make {}", dir.display()))?;
    }
    std::fs::write(at, credential).with_context(|| format!("write {}", at.display()))?;
    std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("keep {} to root alone", at.display()))?;
    // The copy an earlier build kept is done with, the files being open.
    let _ = std::fs::remove_file(OLD_FILE);
    Ok(())
    })
}

/// Forget the screen lock kept from before, if there is one.
pub fn forget() -> Result<()> {
    writable(|| match std::fs::remove_file(FILE) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {FILE}")),
    })
}

/// Where the mark that the switch was turned off lies: a phone that has one
/// keeps no lock, and keeps none at the next start either.
pub const OPTED_OUT: &str = "/metadata/skyward/no-remember";

/// Leave the mark that the switch was turned off.
pub fn opt_out() -> std::io::Result<()> {
    let _ = skyward_block::mount::remount_writable(std::path::Path::new("/metadata"), true);
    let written = std::fs::write(OPTED_OUT, b"");
    let _ = skyward_block::mount::remount_writable(std::path::Path::new("/metadata"), false);
    written
}

/// Take the mark away: the switch was turned on.
pub fn opt_in() -> std::io::Result<()> {
    let _ = skyward_block::mount::remount_writable(std::path::Path::new("/metadata"), true);
    let taken = std::fs::remove_file(OPTED_OUT).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) });
    let _ = skyward_block::mount::remount_writable(std::path::Path::new("/metadata"), false);
    taken
}
