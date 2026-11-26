//! What Skyward keeps of the systems, on /metadata, which the recovery
//! reads before the data partition opens and which no filesystem it holds
//! the areas in can take away without it.

use crate::boot::Start;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use skyward_block::area::Area;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

/// Where the state is kept.
pub const PATH: &str = "/metadata/skyward/systems.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Where the boot sets are kept, once a system has been installed.
    pub store: Option<Store>,
    /// The installed system's boot set, as last saved before the other
    /// system's took its place.
    pub saved: Option<Saved>,
    pub system: Option<System>,
    /// The slot whose boot partitions hold the other system's boot set;
    /// none while they hold the installed system's.
    pub occupied: Option<String>,
    /// What those partitions held once the other system's boot set was
    /// written into them, which tells whether anything else has written
    /// them since. A state kept by a Skyward that did not keep this has
    /// none.
    #[serde(default)]
    pub written: Option<Written>,
    /// A switch begun and not yet done.
    pub switching: Option<Switching>,
    /// Files owned by an installation that has not committed a system yet.
    #[serde(default)]
    pub installing: Option<Installing>,
}

/// The SHA-256 of each of a slot's boot partitions, whole, as they read
/// back once written. The init_boot image written for the other system is
/// made with the program of the Skyward that writes it, so its sum is none
/// the store keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Written {
    pub boot: String,
    pub init_boot: String,
}

/// The area the boot sets are kept in, and how large the partitions they
/// fill are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub area: Area,
    /// The boot partition's bytes.
    pub boot: u64,
    /// The init_boot partition's bytes.
    pub init_boot: u64,
}

/// A boot set in an entry of the store, by the SHA-256 of each partition's
/// whole bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub entry: u32,
    pub boot: String,
    pub init_boot: String,
}

/// The other system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct System {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The filesystem its root is, as the kernel names it.
    pub filesystem: String,
    /// The program its root starts with.
    pub init: String,
    pub root: Area,
    pub set: Saved,
    /// The bytes of the system's own ramdisk, which comes first in the
    /// ramdisk of its init_boot image, where it brings one.
    pub own_ramdisk: Option<u64>,
    /// The SHA-256 of the installed system's vendor_boot and dtbo when the
    /// other system was installed, which the other system starts with.
    pub vendor_boot: String,
    pub dtbo: String,
}

impl System {
    /// What Skyward's program reads of the system, to start it.
    pub fn start(&self) -> Start {
        Start { id: self.id.clone(), filesystem: self.filesystem.clone(), init: self.init.clone(), root: self.root.clone() }
    }
}

/// An installation owns only names derived from this random transaction id.
/// Its intent is durable before either file is created, even before an Area
/// header or extent list exists. A previously committed store is never owned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installing {
    pub transaction: String,
    pub new_store: bool,
}

impl Installing {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.transaction.len() == 32 && self.transaction.bytes().all(|b| b.is_ascii_hexdigit()), "an installation transaction is 32 hexadecimal digits");
        Ok(())
    }

    pub fn root_file(&self, folder: &Path) -> std::path::PathBuf {
        folder.join(format!("root-{}.img", self.transaction))
    }

    pub fn store_file(&self, folder: &Path) -> std::path::PathBuf {
        folder.join(format!("store-{}", self.transaction))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switching {
    pub to: Who,
    /// The slot whose boot partitions are being written.
    pub slot: String,
    /// The installed system's boot set has been saved for this switch, so
    /// the partitions are no longer its to save again.
    pub saved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Who {
    /// The system in the phone's own partitions.
    Installed,
    /// The system in the data partition's area.
    Other,
}

impl Who {
    /// How a request in misc names it: the installed system by a word, the
    /// other by its id.
    pub fn named(name: &str, state: &State) -> Option<Who> {
        match (name, &state.system) {
            ("installed", _) => Some(Who::Installed),
            (id, Some(system)) if system.id == id => Some(Who::Other),
            _ => None,
        }
    }
}

impl State {
    /// The state kept at `path`; none kept is none installed.
    pub fn load(path: &Path) -> Result<State> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("read {}", path.display())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    /// Keep the state at `path` whole or not at all: written beside it,
    /// synced, put in its place, and the folder synced.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.save_with_directory_sync(path, |dir| File::open(dir)?.sync_all())
    }

    fn save_with_directory_sync(&self, path: &Path, mut sync: impl FnMut(&Path) -> std::io::Result<()>) -> Result<()> {
        let dir = path.parent().context("the state is kept in a folder")?;
        fs::create_dir_all(dir).with_context(|| format!("make {}", dir.display()))?;
        // Retry ancestor syncs even when the folder exists: an earlier save
        // may have created it but stopped before its parent was durable.
        // Stop at this filesystem's root; /metadata does not need the
        // ramdisk's root directory synced to persist its own entries.
        let device = fs::metadata(dir)?.dev();
        let mut directory = Some(dir);
        while let Some(at) = directory {
            directory = at.parent();
            let at = if at.as_os_str().is_empty() { Path::new(".") } else { at };
            if fs::metadata(at)?.dev() != device {
                break;
            }
            sync(at).with_context(|| format!("sync {}", at.display()))?;
        }
        let text = toml::to_string(self).context("write the systems' state")?;
        let tmp = path.with_extension("toml.new");
        let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
        f.write_all(text.as_bytes()).with_context(|| format!("write {}", tmp.display()))?;
        f.sync_all().with_context(|| format!("sync {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("put {} in place of {}", tmp.display(), path.display()))?;
        sync(dir).with_context(|| format!("sync {}", dir.display()))
    }

    /// Which system's boot set the boot partitions of `slot` hold.
    pub fn occupant(&self, slot: &str) -> Who {
        match &self.occupied {
            Some(s) if s == slot => Who::Other,
            _ => Who::Installed,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn area(file: &str, size: u64) -> Area {
        Area { partition: "userdata".into(), file: file.into(), id: "00112233445566778899aabbccddeeff".into(), size, extents: vec![(1 << 20, size)] }
    }

    #[test]
    fn an_existing_state_folder_retries_a_failed_ancestor_sync_before_committing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("skyward/systems.toml");
        let state = State { installing: Some(Installing { transaction: "00112233445566778899aabbccddeeff".into(), new_store: true }), ..State::default() };
        let mut attempted = Vec::new();
        let failed = state.save_with_directory_sync(&path, |dir| {
            attempted.push(dir.to_path_buf());
            if dir == root.path() {
                return Err(std::io::Error::other("ancestor sync failed"));
            }
            File::open(dir)?.sync_all()
        });
        assert!(failed.is_err());
        assert!(path.parent().unwrap().is_dir(), "creation succeeded before the sync failed");
        assert!(!path.exists(), "no state is committed before its directory chain is durable");
        assert!(attempted.contains(&root.path().to_path_buf()));
        attempted.clear();
        state.save_with_directory_sync(&path, |dir| {
            attempted.push(dir.to_path_buf());
            File::open(dir)?.sync_all()
        }).unwrap();
        assert!(attempted.contains(&root.path().to_path_buf()), "retry must sync the ancestor even though the state folder exists");
        assert_eq!(State::load(&path).unwrap(), state);
    }

    #[test]
    fn the_state_is_kept_whole_and_read_back_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skyward/systems.toml");
        assert_eq!(State::load(&path).unwrap(), State::default(), "nothing kept, nothing installed");
        let state = State {
            store: Some(Store { area: area("/data/skyward/systems/store", 4096 + 3 * 8192), boot: 4096, init_boot: 4096 }),
            saved: Some(Saved { entry: 1, boot: "aa".into(), init_boot: "bb".into() }),
            system: Some(System {
                id: "test".into(),
                name: "Test".into(),
                version: "1".into(),
                filesystem: "ext4".into(),
                init: "/sbin/init".into(),
                root: area("/data/skyward/systems/test", 1 << 20),
                set: Saved { entry: 2, boot: "cc".into(), init_boot: "dd".into() },
                own_ramdisk: Some(4096),
                vendor_boot: "ee".into(),
                dtbo: "ff".into(),
            }),
            occupied: Some("_a".into()),
            written: Some(Written { boot: "cc".into(), init_boot: "11".into() }),
            switching: Some(Switching { to: Who::Installed, slot: "_a".into(), saved: false }),
            installing: None,
        };
        state.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), state);
        // A state an earlier Skyward kept says nothing of what was written,
        // and reads as one that has none.
        let earlier = State { written: None, ..state.clone() };
        earlier.save(&path).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("written"));
        assert_eq!(State::load(&path).unwrap(), earlier);
        assert!(!path.with_extension("toml.new").exists(), "nothing left beside it");
        assert_eq!((state.occupant("_a"), state.occupant("_b")), (Who::Other, Who::Installed));
        assert_eq!(Who::named("installed", &state), Some(Who::Installed));
        assert_eq!(Who::named("test", &state), Some(Who::Other));
        assert_eq!(Who::named("other", &state), None);
    }
}
