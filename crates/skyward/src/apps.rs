//! The apps the phone keeps in its system partitions: what each one is, the
//! picture it shows, and the copies kept of them.
//!
//! An app is a folder holding an apk; the recovery reads it after mounting the
//! partition it lives in. A copy keeps the folder as it is, so putting it back
//! is copying it back, and deleting one is moving it out of the way rather
//! than losing it: it stays under [`TRASH`] until it is let go of.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Where copies of the apps go, from the top of internal storage.
pub const STORE: &str = "/sdcard/Skyward/apps";

/// Where the apps taken off the phone wait to be put back.
pub const TRASH: &str = "/sdcard/Skyward/apps/trash";

/// The folders under a partition that hold apps.
const KINDS: [&str; 2] = ["app", "priv-app"];

/// One page of what a list shows.
pub const PAGE: usize = 8;

/// An app the phone keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// What the row shows, such as `system/priv-app/Settings`.
    pub label: String,
    /// The partition it lives in, such as `system`.
    pub partition: String,
    /// Its own folder, in the mounted partition.
    pub dir: PathBuf,
    /// The bytes it takes.
    pub bytes: u64,
    /// The picture it shows, where one was found in its apk.
    pub icon: Option<Vec<u8>>,
}

impl App {
    /// Where a copy of it is kept.
    pub fn copy(&self) -> PathBuf {
        Path::new(STORE).join(&self.partition).join(&self.label[self.label.find('/').map_or(0, |at| at + 1)..])
    }

    /// Where it waits after being taken off the phone.
    pub fn held(&self) -> PathBuf {
        Path::new(TRASH).join(&self.partition).join(&self.label[self.label.find('/').map_or(0, |at| at + 1)..])
    }
}

/// The apps in the partitions mounted at `roots`, each with the bytes it takes
/// and the picture it shows.
pub fn list(roots: &[(String, PathBuf)]) -> Result<Vec<App>> {
    let mut out = Vec::new();
    for (partition, root) in roots {
        for kind in KINDS {
            let folder = root.join(kind);
            let Ok(entries) = fs::read_dir(&folder) else { continue };
            for entry in entries.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let Ok(apk) = apk_in(&dir) else { continue };
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.push(App {
                    label: format!("{partition}/{kind}/{name}"),
                    partition: partition.clone(),
                    bytes: measure(&dir),
                    icon: icon_in(&apk).ok().flatten(),
                    dir,
                });
            }
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(out)
}

/// The apk in an app's folder, which is the one named after it where the
/// folder holds more than one.
fn apk_in(dir: &Path) -> Result<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("apk")) {
            found.push(path);
        }
    }
    found.sort();
    found.into_iter().next().with_context(|| format!("no apk in {}", dir.display()))
}

/// The bytes under `dir`, counted.
fn measure(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    let mut total = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.metadata() {
            Ok(m) if m.is_dir() => total += measure(&path),
            Ok(m) => total += m.len(),
            Err(_) => {}
        }
    }
    total
}

/// The picture an apk shows: the largest `ic_launcher` png in it, which is the
/// name an app's own icon is stored under.
fn icon_in(apk: &Path) -> Result<Option<Vec<u8>>> {
    let file = fs::File::open(apk).with_context(|| format!("open {}", apk.display()))?;
    let mut zip = zip::ZipArchive::new(file).with_context(|| format!("read {}", apk.display()))?;
    let mut best: Option<(u64, Vec<u8>)> = None;
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else { continue };
        let name = entry.name().to_ascii_lowercase();
        if !name.ends_with(".png") || !name.contains("ic_launcher") {
            continue;
        }
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut entry, &mut bytes).is_err() {
            continue;
        }
        if best.as_ref().is_none_or(|(size, _)| bytes.len() as u64 > *size) {
            best = Some((bytes.len() as u64, bytes));
        }
    }
    Ok(best.map(|(_, bytes)| bytes))
}

/// Copy the app's folder into the store, as it is.
pub fn copy(app: &App) -> Result<PathBuf> {
    let to = app.copy();
    copy_tree(&app.dir, &to)?;
    Ok(to)
}

/// Put the app's folder back where it was, from the store.
pub fn restore(app: &App) -> Result<()> {
    let from = app.copy();
    copy_tree(&from, &app.dir)
}

/// Take the app off the phone, keeping it to be put back.
pub fn remove(app: &App) -> Result<PathBuf> {
    let to = app.held();
    copy_tree(&app.dir, &to)?;
    fs::remove_dir_all(&app.dir).with_context(|| format!("take away {}", app.dir.display()))?;
    Ok(to)
}

/// Copy `from` to `to`, making `to` and the folders between as they are
/// needed, and keeping what each file had of its own: its mode, and the user
/// and group that own it, so that a system app put back is the one the system
/// expects.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    fs::create_dir_all(to).with_context(|| format!("make {}", to.display()))?;
    fs::set_permissions(to, fs::Permissions::from_mode(fs::metadata(from)?.permissions().mode()))?;
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))?.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let meta = entry.metadata().with_context(|| format!("look at {}", src.display()))?;
        if meta.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            fs::copy(&src, &dst).with_context(|| format!("copy {} to {}", src.display(), dst.display()))?;
            fs::set_permissions(&dst, fs::Permissions::from_mode(meta.permissions().mode()))?;
            let _ = std::os::unix::fs::chown(&dst, Some(meta.uid()), Some(meta.gid()));
        }
    }
    Ok(())
}