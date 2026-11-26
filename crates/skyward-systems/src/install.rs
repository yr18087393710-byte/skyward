//! Installing the other system from its package, switching between the
//! systems on the phone, and taking the other system away again.

use crate::state::{State, Store};
use crate::{Held, Named, Other, Overview};
use skyward_block::area::HEADER;

/// Where the areas' files are, in the data partition.
pub const FOLDER: &str = "/data/skyward/systems";
/// The devices the store and a root are laid out as while they are written.
pub const STORE_DEVICE: &str = "skyward-store";
pub const ROOT_DEVICE: &str = "skyward-root";
/// The gigabyte the pages count sizes in.
const GB: u64 = 1_000_000_000;
/// The sizes a root is offered in, in gigabytes, past what it least takes.
const OFFERED: [u64; 6] = [8, 16, 32, 64, 128, 256];

/// How far an install has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Setting room aside in the data partition: the bytes so far, of all.
    Allocating { done: u64, total: u64 },
    /// Writing the root image, then reading it back.
    Writing { done: u64, total: u64 },
    /// Putting the boot set in the store.
    Finishing,
}

/// The room the data partition has for a system's root, of `free` bytes
/// free: less the store where none is made yet, and less a super's worth,
/// which the installed system's updates keep for themselves, as Android
/// keeps it from its dynamic systems.
pub fn room(free: u64, store: u64, super_size: u64) -> u64 {
    free.saturating_sub(store + super_size + HEADER)
}

/// The sizes offered for a root that takes at least `least` bytes, with
/// `room` bytes to spare: what it least takes in whole gigabytes, and the
/// usual sizes past that, as far as there is room. The area made for one
/// is rounded up to its filesystem's blocks.
pub fn sizes(least: u64, room: u64) -> Vec<u64> {
    let first = least.div_ceil(GB).max(1);
    std::iter::once(first).chain(OFFERED.into_iter().filter(|&s| s > first)).map(|s| s * GB).filter(|&s| s <= room).collect()
}

/// What the pages show of the systems, from the state and the installed
/// system's name; `changed` says whether its vendor_boot or dtbo differ
/// from when the other system was installed, and `held` what the boot
/// partitions hold.
pub fn overview(state: &State, installed: Named, changed: bool, held: Held) -> Overview {
    Overview {
        installed,
        other: state.system.as_ref().map(|s| Other { id: s.id.clone(), name: s.name.clone(), version: s.version.clone(), size: s.root.size - HEADER }),
        other_starts: state.occupied.is_some(),
        changed,
        held,
    }
}

impl Store {
    /// A store's bytes for the phone's partitions, where the store is not
    /// made yet; none where it is.
    pub fn needed(state: &State, boot: u64, init_boot: u64) -> u64 {
        match state.store {
            Some(_) => 0,
            None => Store::size_for(boot, init_boot),
        }
    }
}

/// Persist ownership before allocation. Transaction-specific names keep an
/// interrupted installation separate from both committed areas and old files.
#[cfg(any(target_os = "android", test))]
fn begin_install(state: &mut State, path: &std::path::Path, folder: &std::path::Path, installing: crate::state::Installing) -> anyhow::Result<()> {
    use anyhow::{ensure, Context};
    installing.validate()?;
    ensure!(state.installing.is_none() && state.system.is_none() && state.switching.is_none(), "another system operation is in progress");
    ensure!(installing.new_store == state.store.is_none(), "the installation's store ownership disagrees with the state");
    let mut files = vec![installing.root_file(folder)];
    if installing.new_store {
        files.push(installing.store_file(folder));
    }
    for file in files {
        match std::fs::symlink_metadata(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("look for {}", file.display())),
            Ok(_) => anyhow::bail!("an installation file exists already: {}", file.display()),
        }
    }
    state.installing = Some(installing);
    state.save(path)
}

/// Retry cleanup from the persisted state, including after an uncertain final
/// save: a commit whose rename succeeded must never have its files removed.
/// A failed removal or state save leaves the journal available for another try.
#[cfg(any(target_os = "android", test))]
fn recover_install(path: &std::path::Path, folder: &std::path::Path, mut remove: impl FnMut(&std::path::Path) -> anyhow::Result<()>) -> anyhow::Result<bool> {
    use anyhow::ensure;
    let mut state = State::load(path)?;
    let Some(installing) = state.installing.as_ref() else { return Ok(false) };
    installing.validate()?;
    ensure!(state.system.is_none() && state.switching.is_none() && state.occupied.is_none(), "an unfinished installation conflicts with a committed system or switch");
    ensure!(installing.new_store == state.store.is_none(), "an unfinished installation's store ownership disagrees with the state");
    let root = installing.root_file(folder);
    ensure!(state.store.as_ref().is_none_or(|s| s.area.file != root), "an unfinished installation cannot remove the committed store");
    remove(&root)?;
    if installing.new_store {
        remove(&installing.store_file(folder))?;
    }
    state.installing = None;
    state.save(path)?;
    Ok(true)
}

#[cfg(target_os = "android")]
pub use sys::*;

#[cfg(target_os = "android")]
mod sys {
    use super::*;
    use crate::boot::{self, Start};
    use crate::package::{Contents, BOOT, INIT_BOOT, ROOT};
    use anyhow::Result;
    use crate::state::{System, Who, PATH};
    use crate::store::{self, OTHER};
    use crate::switch::{self, Places};
    use anyhow::{ensure, Context};
    use sha2::{Digest, Sha256};
    use skyward_block::area::{self, Area};
    use skyward_block::{dm, BY_NAME};
    use skyward_vab::Slot;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};
    use zip::ZipArchive;

    const CHUNK: usize = 1 << 20;

    /// A failed write can leave its mapper behind if removal was busy. Do
    /// not free the underlying file until every installation mapper is gone.
    fn remove_unfinished_area(path: &Path) -> Result<()> {
        for name in [ROOT_DEVICE, STORE_DEVICE] {
            if dm::status(name)?.is_some() {
                dm::remove(name).with_context(|| format!("remove {name} before reclaiming an unfinished installation"))?;
            }
        }
        Area::remove_path(path)
    }

    fn by_name() -> &'static Path {
        Path::new(BY_NAME)
    }

    /// Where the data partition is mounted.
    const DATA: &str = "/data";

    fn data() -> &'static Path {
        Path::new(DATA)
    }

    /// The bytes free in the data partition's filesystem.
    pub fn free() -> Result<u64> {
        Ok(skyward_block::mount::space(data()).context("look at the room in the data partition")?.available)
    }

    /// Reclaim only files owned by an interrupted install, before computing
    /// the offered root sizes. An empty journal needs no writable metadata.
    pub fn prepare_install() -> Result<bool> {
        if state()?.installing.is_none() {
            return Ok(false);
        }
        ensure!(skyward_block::mount::is_mount_point(data())?, "the data partition is not open");
        writable(|| recover_install(Path::new(PATH), Path::new(FOLDER), remove_unfinished_area))
    }

    /// The room for a root, in the data partition as it is now. Call
    /// `prepare_install` first when offering sizes for a new installation.
    pub fn room_now(slot: Slot) -> Result<u64> {
        let state = state()?;
        let size = |name: &str| skyward_block::size_bytes(store::partition(by_name(), name, slot));
        let needed = Store::needed(&state, size(store::BOOT)?, size(store::INIT_BOOT)?);
        Ok(room(free()?, needed, skyward_block::size_bytes(by_name().join("super"))?))
    }

    /// Lay the store out as a device, once it is found where it was, run
    /// `work` on it, and take the device away again.
    fn with_store<T>(state: &State, work: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
        let store = state.store.as_ref().context("no store holds the boot sets")?;
        store.area.check_header()?;
        if dm::status(STORE_DEVICE)?.is_some() {
            dm::remove(STORE_DEVICE)?;
        }
        let node = store.area.map(STORE_DEVICE)?;
        let done = work(&node);
        let gone = dm::remove(STORE_DEVICE);
        let done = done?;
        gone?;
        Ok(done)
    }

    const METADATA: &str = "/metadata";

    /// Mount /metadata, where the state is kept, read-only where it is not
    /// mounted yet, as the recovery keeps it.
    pub fn metadata() -> Result<()> {
        let at = Path::new(METADATA);
        if !skyward_block::mount::is_mount_point(at)? {
            skyward_block::mount::mount_ro(&by_name().join("metadata"), at)?;
        }
        Ok(())
    }

    /// Run `work` with /metadata writable, and leave it read-only after.
    fn writable<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
        metadata()?;
        let at = Path::new(METADATA);
        skyward_block::mount::remount_writable(at, true)?;
        let done = work();
        let back = skyward_block::mount::remount_writable(at, false);
        let done = done?;
        back?;
        Ok(done)
    }

    /// The state as kept now.
    pub fn state() -> Result<State> {
        metadata()?;
        State::load(Path::new(PATH))
    }

    /// The program the other system starts with: this recovery's own.
    fn program() -> PathBuf {
        Path::new("/").join(skyward_config::ramdisk::SYSTEM_INIT)
    }

    /// Have the bootloader start `to`, writing its boot set into the boot
    /// partitions; see [`switch::switch`]. Switching to the other system
    /// waits for any update to be merged, and first checks that the other
    /// system's root is still where it was put.
    pub fn switch_to(next: Slot, to: Who) -> Result<()> {
        let state = state()?;
        if to == Who::Other {
            let update = skyward_vab::snapshot_state()?;
            ensure!(update == skyward_vab::SnapshotState::None, "an update is in flight ({update:?}), and the installed system has to start once more to merge it");
            state.system.as_ref().context("no other system is installed")?.root.check_header()?;
        }
        let (path, program) = (Path::new(PATH), program());
        writable(|| with_store(&state, |device| switch::switch(&Places { state: path, by_name: by_name(), store: device, program: &program }, next, to)))
    }

    /// Finish a switch cut off before, where one was; says whether there
    /// was one.
    pub fn resume() -> Result<bool> {
        let state = state()?;
        if state.switching.is_none() {
            return Ok(false);
        }
        let (path, program) = (Path::new(PATH), program());
        writable(|| with_store(&state, |device| switch::resume(&Places { state: path, by_name: by_name(), store: device, program: &program })))
    }

    /// What the boot partitions hold, set against the state as kept now;
    /// `starts` is the slot the bootloader starts.
    pub fn held(state: &State, starts: Slot) -> Result<Held> {
        switch::held(state, by_name(), starts)
    }

    /// Put the installed system's boot set back where the other system's
    /// is, as everything that writes or erases the installed system first
    /// does; says whether it had to. Where something else was written into
    /// those partitions since, nobody asked for it to be written over, and
    /// nothing is: the one who switches to the installed system on the
    /// systems page, told of it there, asks for that.
    pub fn ensure_installed() -> Result<bool> {
        resume()?;
        let state = state()?;
        let Some(slot) = state.occupied.as_deref().map(|s| Slot::from_suffix(s).with_context(|| format!("the other system is in slot {s:?}"))).transpose()? else {
            return Ok(false);
        };
        ensure!(
            held(&state, slot)? != Held::Rewritten,
            "the boot partitions of slot {} no longer hold the second system's boot set as Skyward wrote it, and the main system's is not written over what is there unasked: switch to the main system in Dual boot first",
            slot.suffix()
        );
        switch_to(slot, Who::Installed)?;
        Ok(true)
    }

    /// The SHA-256 of all of a partition.
    fn sum_of(path: &Path) -> Result<String> {
        let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        skyward_block::drop_cache(&f)?;
        let mut sum = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf).with_context(|| format!("read {}", path.display()))?;
            if n == 0 {
                break;
            }
            sum.update(&buf[..n]);
        }
        Ok(hex::encode(sum.finalize()))
    }

    /// Whether the installed system's vendor_boot or dtbo in `slot` differ
    /// from when the other system was installed.
    pub fn changed(state: &State, slot: Slot) -> Result<bool> {
        let Some(system) = &state.system else { return Ok(false) };
        Ok(sum_of(&store::partition(by_name(), "vendor_boot", slot))? != system.vendor_boot
            || sum_of(&store::partition(by_name(), "dtbo", slot))? != system.dtbo)
    }

    /// Install the system in the package `zip`, whose contents are
    /// `contents`, with a root of `size` bytes: its root written into a
    /// new area of the data partition, which has to be open, and its boot
    /// set, with `program` to start it, put in the store, made where there
    /// is none yet. `slot` is the slot the phone runs from, whose
    /// init_boot image lends its header.
    pub fn install<R: Read + Seek>(zip: &mut ZipArchive<R>, contents: &Contents, size: u64, program: &[u8], slot: Slot, progress: &mut dyn FnMut(Step)) -> Result<()> {
        writable(|| install_now(zip, contents, size, program, slot, progress))
    }

    fn install_now<R: Read + Seek>(zip: &mut ZipArchive<R>, contents: &Contents, size: u64, program: &[u8], slot: Slot, progress: &mut dyn FnMut(Step)) -> Result<()> {
        let path = Path::new(PATH);
        recover_install(path, Path::new(FOLDER), remove_unfinished_area)?;
        let mut state = State::load(path)?;
        ensure!(state.switching.is_none(), "a switch begun before is not done yet");
        ensure!(state.system.is_none(), "another system is installed already");
        ensure!(size >= contents.least(), "a root of {size} bytes is less than the {} the system takes", contents.least());
        let room = room_now(slot)?;
        ensure!(size <= room, "the data partition has room for {room} bytes, less than {size}");
        let folder = Path::new(FOLDER);
        fs::create_dir_all(folder).with_context(|| format!("make {}", folder.display()))?;
        // The journal is on another filesystem. Persist the newly made
        // folders on /data before it can name files beneath them.
        let mut directory = Some(folder);
        while let Some(at) = directory.filter(|at| at.starts_with(data())) {
            File::open(at)?.sync_all().with_context(|| format!("sync {}", at.display()))?;
            directory = at.parent();
        }
        let installing = crate::state::Installing { transaction: area::new_id()?, new_store: state.store.is_none() };
        begin_install(&mut state, path, folder, installing.clone())?;
        let id = contents.manifest.id.clone();
        let made = (|| -> Result<System> {
            if installing.new_store {
                let (boot, init_boot) = (skyward_block::size_bytes(store::partition(by_name(), store::BOOT, slot))?, skyward_block::size_bytes(store::partition(by_name(), store::INIT_BOOT, slot))?);
                let area = area::create(&installing.store_file(folder), Store::size_for(boot, init_boot), &mut |done, total| progress(Step::Allocating { done, total }))?;
                state.store = Some(Store { area, boot, init_boot });
            }
            let root = area::create(&installing.root_file(folder), size + HEADER, &mut |done, total| progress(Step::Allocating { done, total }))?;
            write_root(zip, &root, contents.root, progress)?;
            progress(Step::Finishing);
            let start = Start { id: id.clone(), filesystem: contents.manifest.filesystem.clone(), init: contents.manifest.init.clone(), root: root.clone() };
            let (init_boot, own_ramdisk) = match contents.init_boot {
                true => {
                    let mut own = Vec::new();
                    zip.by_name(INIT_BOOT).context("open init_boot.img")?.read_to_end(&mut own).context("read init_boot.img")?;
                    let (image, own_ramdisk) = boot::with_start(&own, program, &start)?;
                    (image, Some(own_ramdisk))
                }
                false => (boot::init_boot(&fs::read(store::partition(by_name(), store::INIT_BOOT, slot)).context("read the phone's init_boot")?, program, &start)?, None),
            };
            let set = with_store(&state, |device| {
                let mut image = zip.by_name(BOOT).context("open boot.img")?;
                let store = state.store.as_ref().expect("made above");
                store::put(store, device, OTHER, &mut image, contents.boot, &init_boot)
            })?;
            Ok(System {
                id: id.clone(),
                name: contents.manifest.name.clone(),
                version: contents.manifest.version.clone(),
                filesystem: contents.manifest.filesystem.clone(),
                init: contents.manifest.init.clone(),
                root: root.clone(),
                set,
                own_ramdisk,
                vendor_boot: sum_of(&store::partition(by_name(), "vendor_boot", slot))?,
                dtbo: sum_of(&store::partition(by_name(), "dtbo", slot))?,
            })
        })();
        let committed = made.and_then(|system| {
            state.system = Some(system);
            state.installing = None;
            state.save(path)
        });
        match committed {
            Ok(()) => Ok(()),
            Err(e) => match recover_install(path, folder, remove_unfinished_area) {
                Ok(_) => Err(e),
                Err(r) => Err(e.context(format!("taking the unfinished installation away failed too: {r:#}"))),
            },
        }
    }

    /// Write the root image from the package into the root area, after its
    /// header, then read it back.
    fn write_root<R: Read + Seek>(zip: &mut ZipArchive<R>, root: &Area, len: u64, progress: &mut dyn FnMut(Step)) -> Result<()> {
        ensure!(len <= root.size - HEADER, "the root image takes {len} bytes, more than its area's {}", root.size - HEADER);
        if dm::status(ROOT_DEVICE)?.is_some() {
            dm::remove(ROOT_DEVICE)?;
        }
        let node = root.map(ROOT_DEVICE)?;
        let written = (|| -> Result<()> {
            let mut out = OpenOptions::new().read(true).write(true).open(&node).with_context(|| format!("open {}", node.display()))?;
            let mut image = zip.by_name(ROOT).context("open rootfs.img")?;
            out.seek(SeekFrom::Start(HEADER))?;
            let mut buf = vec![0u8; CHUNK];
            let (mut sum, mut done) = (Sha256::new(), 0);
            while done < len {
                let n = (len - done).min(CHUNK as u64) as usize;
                image.read_exact(&mut buf[..n]).context("read rootfs.img")?;
                out.write_all(&buf[..n]).context("write the root")?;
                sum.update(&buf[..n]);
                done += n as u64;
                progress(Step::Writing { done, total: len * 2 });
            }
            out.sync_all().context("sync the root")?;
            skyward_block::drop_cache(&out)?;
            out.seek(SeekFrom::Start(HEADER))?;
            let (mut back, mut read) = (Sha256::new(), 0);
            while read < len {
                let n = (len - read).min(CHUNK as u64) as usize;
                out.read_exact(&mut buf[..n]).context("read the root back")?;
                back.update(&buf[..n]);
                read += n as u64;
                progress(Step::Writing { done: len + read, total: len * 2 });
            }
            ensure!(back.finalize() == sum.finalize(), "the root reads back different from what was written");
            Ok(())
        })();
        let gone = dm::remove(ROOT_DEVICE);
        written?;
        gone
    }

    /// Take the other system away: its boot set out of the partitions
    /// where it is there, its root and the store out of the data partition,
    /// which has to be open, and the state with them.
    pub fn remove() -> Result<()> {
        ensure!(skyward_block::mount::is_mount_point(data())?, "the data partition is not open");
        writable(|| recover_install(Path::new(PATH), Path::new(FOLDER), remove_unfinished_area))?;
        ensure_installed()?;
        writable(|| {
            let path = Path::new(PATH);
            let mut state = State::load(path)?;
            if let Some(system) = &state.system {
                system.root.remove()?;
                state.system = None;
                state.save(path)?;
            }
            if let Some(store) = &state.store {
                store.area.remove()?;
                state = State::default();
                state.save(path)?;
            }
            let folder = PathBuf::from(FOLDER);
            if folder.exists() && fs::read_dir(&folder)?.next().is_none() {
                fs::remove_dir(&folder).with_context(|| format!("delete {}", folder.display()))?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(new_store: bool) -> crate::state::Installing {
        crate::state::Installing { transaction: "00112233445566778899aabbccddeeff".into(), new_store }
    }

    fn remove_test_file(path: &std::path::Path) -> anyhow::Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    #[test]
    fn an_install_intent_survives_each_interrupted_allocation_stage() {
        for stage in 0..=3 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("systems.toml");
            let mut state = State::default();
            let pending = intent(true);
            begin_install(&mut state, &path, dir.path(), pending.clone()).unwrap();
            assert_eq!(State::load(&path).unwrap().installing, Some(pending.clone()), "ownership precedes allocation");
            if stage >= 1 {
                std::fs::write(pending.store_file(dir.path()), b"partial store").unwrap();
            }
            if stage >= 2 {
                std::fs::write(pending.root_file(dir.path()), if stage == 2 { b"partial".as_slice() } else { b"whole root".as_slice() }).unwrap();
            }
            let unrelated = dir.path().join("old-version-orphan.img");
            std::fs::write(&unrelated, b"leave alone").unwrap();
            assert!(recover_install(&path, dir.path(), remove_test_file).unwrap());
            assert_eq!(State::load(&path).unwrap(), State::default());
            assert!(!pending.root_file(dir.path()).exists());
            assert!(!pending.store_file(dir.path()).exists());
            assert_eq!(std::fs::read(unrelated).unwrap(), b"leave alone");
            assert!(!recover_install(&path, dir.path(), |_| panic!("already recovered")).unwrap());
            begin_install(&mut State::default(), &path, dir.path(), pending).unwrap();
        }
    }

    #[test]
    fn failed_cleanup_keeps_the_intent_until_every_file_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("systems.toml");
        let pending = intent(true);
        begin_install(&mut State::default(), &path, dir.path(), pending.clone()).unwrap();
        std::fs::write(pending.root_file(dir.path()), b"root").unwrap();
        std::fs::write(pending.store_file(dir.path()), b"store").unwrap();
        assert!(recover_install(&path, dir.path(), |file| {
            if file == pending.store_file(dir.path()) { anyhow::bail!("injected removal failure"); }
            remove_test_file(file)
        }).is_err());
        assert_eq!(State::load(&path).unwrap().installing, Some(pending.clone()));
        assert!(!pending.root_file(dir.path()).exists(), "first removal already happened");
        assert!(pending.store_file(dir.path()).exists());
        assert!(recover_install(&path, dir.path(), remove_test_file).unwrap());
        assert!(State::load(&path).unwrap().installing.is_none());
    }

    #[test]
    fn a_busy_cleanup_keeps_both_areas_and_the_journal_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("systems.toml");
        let pending = intent(true);
        begin_install(&mut State::default(), &path, dir.path(), pending.clone()).unwrap();
        let root = pending.root_file(dir.path());
        let store = pending.store_file(dir.path());
        std::fs::write(&root, b"root").unwrap();
        std::fs::write(&store, b"store").unwrap();
        assert!(recover_install(&path, dir.path(), |_| anyhow::bail!("installation mapper is busy")).is_err());
        assert_eq!(std::fs::read(&root).unwrap(), b"root");
        assert_eq!(std::fs::read(&store).unwrap(), b"store");
        assert_eq!(State::load(&path).unwrap().installing, Some(pending));
        assert!(recover_install(&path, dir.path(), remove_test_file).unwrap());
        assert!(!root.exists() && !store.exists());
    }

    #[test]
    fn a_failed_journal_clear_can_retry_after_the_files_are_already_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("systems.toml");
        let pending = intent(true);
        begin_install(&mut State::default(), &path, dir.path(), pending.clone()).unwrap();
        std::fs::write(pending.root_file(dir.path()), b"partial").unwrap();
        let tmp = path.with_extension("toml.new");
        std::fs::create_dir(&tmp).unwrap();
        assert!(recover_install(&path, dir.path(), remove_test_file).is_err());
        assert_eq!(State::load(&path).unwrap().installing, Some(pending.clone()));
        assert!(!pending.root_file(dir.path()).exists());
        std::fs::remove_dir(tmp).unwrap();
        assert!(recover_install(&path, dir.path(), remove_test_file).unwrap());
        assert!(State::load(&path).unwrap().installing.is_none());
    }

    #[test]
    fn recovering_an_install_preserves_the_committed_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("systems.toml");
        let file = dir.path().join("committed-store");
        std::fs::write(&file, b"saved boot sets").unwrap();
        let store = Store { area: crate::state::tests::area(file.to_str().unwrap(), Store::size_for(8192, 16384)), boot: 8192, init_boot: 16384 };
        let mut state = State { store: Some(store.clone()), ..State::default() };
        let pending = intent(false);
        begin_install(&mut state, &path, dir.path(), pending.clone()).unwrap();
        std::fs::write(pending.root_file(dir.path()), b"partial").unwrap();
        assert!(recover_install(&path, dir.path(), remove_test_file).unwrap());
        assert_eq!(State::load(&path).unwrap(), State { store: Some(store), ..State::default() });
        assert_eq!(std::fs::read(file).unwrap(), b"saved boot sets");
    }

    #[test]
    fn cleanup_reloads_the_persisted_commit_instead_of_removing_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("systems.toml");
        let pending = intent(true);
        let mut state = State::default();
        begin_install(&mut state, &path, dir.path(), pending.clone()).unwrap();
        let file = pending.root_file(dir.path());
        std::fs::write(&file, b"committed root").unwrap();
        // The final rename succeeded, even if its caller saw a sync error.
        state.store = Some(Store { area: crate::state::tests::area(pending.store_file(dir.path()).to_str().unwrap(), Store::size_for(8192, 16384)), boot: 8192, init_boot: 16384 });
        state.system = Some(crate::state::System {
            id: "test".into(), name: "Test".into(), version: "1".into(), filesystem: "ext4".into(), init: "/sbin/init".into(),
            root: crate::state::tests::area(file.to_str().unwrap(), 1 << 20),
            set: crate::state::Saved { entry: crate::store::OTHER, boot: "aa".repeat(32), init_boot: "bb".repeat(32) },
            own_ramdisk: None, vendor_boot: "cc".repeat(32), dtbo: "dd".repeat(32),
        });
        state.installing = None;
        state.save(&path).unwrap();
        assert!(!recover_install(&path, dir.path(), |_| panic!("a committed file must be retained")).unwrap());
        assert_eq!(std::fs::read(file).unwrap(), b"committed root");
    }

    #[test]
    fn invalid_or_preexisting_installation_names_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("systems.toml");
        let invalid = crate::state::Installing { transaction: "../outside".into(), new_store: true };
        assert!(begin_install(&mut State::default(), &path, dir.path(), invalid.clone()).is_err());
        assert!(!path.exists());
        std::fs::write(&path, toml::to_string(&State { installing: Some(invalid), ..State::default() }).unwrap()).unwrap();
        assert!(recover_install(&path, dir.path(), |_| panic!("validate before deleting")).is_err());
        let pending = intent(true);
        let file = pending.root_file(dir.path());
        std::fs::write(&file, b"preexisting").unwrap();
        assert!(begin_install(&mut State::default(), &path, dir.path(), pending).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"preexisting");
    }

    #[test]
    fn legacy_state_without_an_install_journal_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("systems.toml");
        std::fs::write(&path, "").unwrap();
        assert_eq!(State::load(&path).unwrap(), State::default());
        assert!(!recover_install(&path, dir.path(), |_| panic!("no journal")).unwrap());
    }

    #[test]
    fn the_room_for_a_root_keeps_a_supers_worth_for_updates() {
        assert_eq!(room(59 * GB, 600 << 20, 9 * GB), 59 * GB - (600 << 20) - 9 * GB - HEADER);
        assert_eq!(room(GB, 0, 9 * GB), 0, "none to spare");
    }

    #[test]
    fn the_sizes_offered_start_at_what_the_root_takes_and_stay_within_room() {
        assert_eq!(sizes(3 << 30, 50 * GB), [4 * GB, 8 * GB, 16 * GB, 32 * GB], "3 GiB is past 3 GB");
        assert_eq!(sizes(100 << 20, 9 * GB), [GB, 8 * GB]);
        assert_eq!(sizes(20 * GB, 300 * GB), [20 * GB, 32 * GB, 64 * GB, 128 * GB, 256 * GB]);
        assert!(sizes(20 * GB, 10 * GB).is_empty(), "no room for it at all");
    }
}
