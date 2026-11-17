//! avbctl as install scripts run it before they write a system that
//! verified boot would turn away: to tell, and to turn on or off,
//! dm-verity, which checks each block of a partition as it is read, and
//! verification, the bootloader's checking of the partitions at all. Each
//! is a flag in the header of the top-level vbmeta image of the slot the
//! phone runs from, which an unlocked bootloader honours. What it prints
//! and how it ends are what those scripts read.

use super::Io;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// How avbctl ends, numbered as sysexits.h numbers them.
const DONE: i32 = 0;
const MISUSED: i32 = 64;
const FAILED: i32 = 70;
const REFUSED: i32 = 77;

/// A vbmeta image's header: 256 bytes starting with its magic, its flags a
/// big-endian 32-bit word 120 bytes in.
const HEADER_LEN: usize = 256;
const MAGIC: &[u8] = b"AVB0";
const FLAGS_AT: usize = 120;

/// The footer ending a partition whose image carries its vbmeta image
/// along: 64 bytes starting with its magic, where the vbmeta image starts
/// in the partition a big-endian 64-bit word 20 bytes in.
const FOOTER_LEN: usize = 64;
const FOOTER_MAGIC: &[u8] = b"AVBf";
const VBMETA_AT: usize = 20;

const USAGE: &str = "\
avbctl - command-line tool for AVB.

Usage:
  avbctl [--force] COMMAND

Commands:
  avbctl get-verity           - Prints whether verity is enabled in current slot.
  avbctl disable-verity       - Disable verity in current slot.
  avbctl enable-verity        - Enable verity in current slot.
  avbctl get-verification     - Prints whether verification is enabled in current slot.
  avbctl disable-verification - Disable verification in current slot.
  avbctl enable-verification  - Enable verification in current slot.
";

const LOCKED: &str = "\
avbctl: the bootloader is locked, and a locked bootloader does not boot a slot
whose vbmeta image has been changed. Run with --force to change it all the same.
";

/// What avbctl turns on and off.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Check {
    Verity,
    Verification,
}

impl Check {
    fn name(self) -> &'static str {
        match self {
            Check::Verity => "verity",
            Check::Verification => "verification",
        }
    }

    /// The flag in the header that turns it off.
    fn off(self) -> u32 {
        match self {
            Check::Verity => 1 << 0,
            Check::Verification => 1 << 1,
        }
    }
}

/// What avbctl is asked: whether a check is on, or to turn it on or off.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Get(Check),
    Set(Check, bool),
}

/// The phone avbctl works on: the folder its partitions are named in, the
/// suffix of the slot it runs from, empty on a phone with one slot, and
/// whether its bootloader is locked.
struct Phone<'a> {
    by_name: &'a Path,
    suffix: &'a str,
    locked: bool,
}

pub(super) fn run(args: &[OsString], io: &mut Io) -> i32 {
    let suffix = skyward_sys::prop("ro.boot.slot_suffix").unwrap_or_default();
    let locked = skyward_sys::prop("ro.boot.vbmeta.device_state").as_deref() == Some("locked");
    avbctl(args, io, &Phone { by_name: Path::new(skyward_block::BY_NAME), suffix: &suffix, locked })
}

fn avbctl(args: &[OsString], io: &mut Io, phone: &Phone) -> i32 {
    let (mut force, mut command) = (false, None);
    // Of the commands given the last counts, and anything else is passed
    // over.
    for arg in args {
        match arg.to_str() {
            Some("--force") => force = true,
            Some("get-verity") => command = Some(Command::Get(Check::Verity)),
            Some("disable-verity") => command = Some(Command::Set(Check::Verity, false)),
            Some("enable-verity") => command = Some(Command::Set(Check::Verity, true)),
            Some("get-verification") => command = Some(Command::Get(Check::Verification)),
            Some("disable-verification") => command = Some(Command::Set(Check::Verification, false)),
            Some("enable-verification") => command = Some(Command::Set(Check::Verification, true)),
            _ => {}
        }
    }
    let Some(command) = command else {
        let _ = io.errors.write_all(USAGE.as_bytes());
        return MISUSED;
    };
    let (Command::Get(check) | Command::Set(check, _)) = command;
    let on = match is_on(phone, check) {
        Ok(on) => on,
        Err(e) => {
            let _ = writeln!(io.errors, "avbctl: {e}");
            let _ = writeln!(io.errors, "Error getting whether {} is enabled.", check.name());
            return FAILED;
        }
    };
    let slot = match phone.suffix {
        "" => String::new(),
        suffix => format!(" on slot with suffix {suffix}"),
    };
    let state = |on: bool| if on { "enabled" } else { "disabled" };
    match command {
        Command::Get(_) => say(io, &format!("{} is {}{slot}.\n", check.name(), state(on))),
        Command::Set(_, want) if want == on => say(io, &format!("{} is already {}{slot}.\n", check.name(), state(on))),
        Command::Set(_, want) => {
            if phone.locked && !force {
                let _ = io.errors.write_all(LOCKED.as_bytes());
                return REFUSED;
            }
            if let Err(e) = set(phone, check, want) {
                let _ = writeln!(io.errors, "avbctl: {e}");
                let _ = writeln!(io.errors, "Error setting {}.", check.name());
                return FAILED;
            }
            say(io, &format!("Successfully {} {}{slot}. Reboot the device for changes to take effect.\n", state(want), check.name()))
        }
    }
}

fn say(io: &mut Io, text: &str) -> i32 {
    match io.output.write_all(text.as_bytes()).and_then(|()| io.output.flush()) {
        Ok(()) => DONE,
        Err(_) => FAILED,
    }
}

/// Whether `check` is on in the slot's top-level vbmeta image.
fn is_on(phone: &Phone, check: Check) -> io::Result<bool> {
    let (path, at) = top_level(phone)?;
    let mut file = File::open(&path).map_err(|e| about(&path, e))?;
    let header = header(&mut file, &path, at)?;
    Ok(flags(&header) & check.off() == 0)
}

/// Turn `check` on or off in the slot's top-level vbmeta image, leaving the
/// rest of it as it was.
fn set(phone: &Phone, check: Check, on: bool) -> io::Result<()> {
    let (path, at) = top_level(phone)?;
    let mut file = OpenOptions::new().read(true).write(true).open(&path).map_err(|e| about(&path, e))?;
    let mut header = header(&mut file, &path, at)?;
    let flags = match on {
        true => flags(&header) & !check.off(),
        false => flags(&header) | check.off(),
    };
    header[FLAGS_AT..FLAGS_AT + 4].copy_from_slice(&flags.to_be_bytes());
    file.seek(SeekFrom::Start(at)).and_then(|_| file.write_all(&header)).and_then(|()| file.sync_all()).map_err(|e| about(&path, e))
}

/// Where the slot's top-level vbmeta image starts: at the start of its
/// vbmeta partition, or, on a phone without one, in its boot partition
/// where the footer ending it says.
fn top_level(phone: &Phone) -> io::Result<(PathBuf, u64)> {
    let vbmeta = phone.by_name.join(format!("vbmeta{}", phone.suffix));
    if vbmeta.exists() {
        return Ok((vbmeta, 0));
    }
    let boot = phone.by_name.join(format!("boot{}", phone.suffix));
    let mut file = File::open(&boot).map_err(|e| about(&boot, e))?;
    let mut footer = [0; FOOTER_LEN];
    file.seek(SeekFrom::End(-(FOOTER_LEN as i64))).and_then(|_| file.read_exact(&mut footer)).map_err(|e| about(&boot, e))?;
    if !footer.starts_with(FOOTER_MAGIC) {
        let why = format!("there is no {}, and {} does not end in a vbmeta footer", vbmeta.display(), boot.display());
        return Err(io::Error::new(ErrorKind::InvalidData, why));
    }
    let at = u64::from_be_bytes(footer[VBMETA_AT..VBMETA_AT + 8].try_into().expect("eight bytes"));
    Ok((boot, at))
}

/// The vbmeta header `at` bytes into `file`, the partition at `path`.
fn header(file: &mut File, path: &Path, at: u64) -> io::Result<[u8; HEADER_LEN]> {
    let mut header = [0; HEADER_LEN];
    file.seek(SeekFrom::Start(at)).and_then(|_| file.read_exact(&mut header)).map_err(|e| about(path, e))?;
    if !header.starts_with(MAGIC) {
        return Err(io::Error::new(ErrorKind::InvalidData, format!("{} holds no vbmeta header {at} bytes in", path.display())));
    }
    Ok(header)
}

fn flags(header: &[u8; HEADER_LEN]) -> u32 {
    u32::from_be_bytes(header[FLAGS_AT..FLAGS_AT + 4].try_into().expect("four bytes"))
}

/// `e`, saying which partition it came from.
fn about(path: &Path, e: io::Error) -> io::Error {
    io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A vbmeta image whose flags are `flags`: its header, and the
    /// authentication block after it, which avbctl leaves as it is.
    fn vbmeta(flags: u32) -> Vec<u8> {
        let mut image = vec![0u8; 256 + 64];
        image[..4].copy_from_slice(b"AVB0");
        image[120..124].copy_from_slice(&flags.to_be_bytes());
        image[128..141].copy_from_slice(b"avbtool 1.3.0");
        for (i, b) in image[256..].iter_mut().enumerate() {
            *b = i as u8 + 1;
        }
        image
    }

    fn ran(args: &[&str], phone: &Phone) -> (i32, String, String) {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        let code = avbctl(&args, &mut Io { input: &mut io::empty(), output: &mut output, errors: &mut errors }, phone);
        (code, String::from_utf8(output).unwrap(), String::from_utf8(errors).unwrap())
    }

    #[test]
    fn verity_and_verification_are_flags_in_the_header_of_the_slot_s_vbmeta_image() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("vbmeta_a"), dir.path().join("vbmeta_b"));
        fs::write(&a, vbmeta(0)).unwrap();
        fs::write(&b, vbmeta(0)).unwrap();
        let phone = Phone { by_name: dir.path(), suffix: "_a", locked: false };
        assert_eq!(ran(&["get-verity"], &phone), (0, "verity is enabled on slot with suffix _a.\n".into(), String::new()));
        assert_eq!(
            ran(&["--force", "disable-verity"], &phone),
            (0, "Successfully disabled verity on slot with suffix _a. Reboot the device for changes to take effect.\n".into(), String::new())
        );
        assert_eq!(fs::read(&a).unwrap(), vbmeta(1), "the verity flag set and nothing else");
        assert_eq!(ran(&["disable-verity"], &phone), (0, "verity is already disabled on slot with suffix _a.\n".into(), String::new()));
        assert_eq!(ran(&["disable-verification", "--force"], &phone).0, 0, "the command and --force in either order");
        assert_eq!(fs::read(&a).unwrap(), vbmeta(3));
        assert_eq!(ran(&["get-verification"], &phone).1, "verification is disabled on slot with suffix _a.\n");
        assert_eq!(ran(&["get-verity"], &phone).1, "verity is disabled on slot with suffix _a.\n");
        assert_eq!(
            ran(&["enable-verity"], &phone).1,
            "Successfully enabled verity on slot with suffix _a. Reboot the device for changes to take effect.\n"
        );
        assert_eq!(fs::read(&a).unwrap(), vbmeta(2));
        assert_eq!(ran(&["enable-verification"], &phone).0, 0);
        assert_eq!(fs::read(&a).unwrap(), vbmeta(0));
        assert_eq!(fs::read(&b).unwrap(), vbmeta(0), "the other slot's as it was");
    }

    #[test]
    fn a_phone_with_one_slot_names_none() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("vbmeta"), vbmeta(2)).unwrap();
        let phone = Phone { by_name: dir.path(), suffix: "", locked: false };
        assert_eq!(ran(&["get-verification"], &phone).1, "verification is disabled.\n");
        assert_eq!(ran(&["enable-verification"], &phone).1, "Successfully enabled verification. Reboot the device for changes to take effect.\n");
        assert_eq!(fs::read(dir.path().join("vbmeta")).unwrap(), vbmeta(0));
    }

    #[test]
    fn a_locked_bootloader_s_flags_change_only_when_forced() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("vbmeta_a");
        fs::write(&a, vbmeta(2)).unwrap();
        let phone = Phone { by_name: dir.path(), suffix: "_a", locked: true };
        assert_eq!(ran(&["disable-verity"], &phone), (77, String::new(), LOCKED.into()));
        assert_eq!(fs::read(&a).unwrap(), vbmeta(2), "nothing written");
        assert_eq!(ran(&["get-verity"], &phone).0, 0, "telling needs no force");
        assert_eq!(ran(&["disable-verification"], &phone).1, "verification is already disabled on slot with suffix _a.\n");
        assert_eq!(ran(&["--force", "disable-verity"], &phone).0, 0);
        assert_eq!(fs::read(&a).unwrap(), vbmeta(3));
    }

    #[test]
    fn without_a_vbmeta_partition_the_image_is_where_the_boot_partition_s_footer_says() {
        let dir = tempfile::tempdir().unwrap();
        let boot = dir.path().join("boot_b");
        let partition = |flags: u32| {
            let (kernel, image) = (vec![0x11u8; 4096], vbmeta(flags));
            let mut boot = kernel.clone();
            boot.extend_from_slice(&image);
            boot.resize(4096 * 3, 0);
            let mut footer = b"AVBf".to_vec();
            footer.extend_from_slice(&1u32.to_be_bytes());
            footer.extend_from_slice(&0u32.to_be_bytes());
            footer.extend_from_slice(&(kernel.len() as u64).to_be_bytes());
            footer.extend_from_slice(&4096u64.to_be_bytes());
            footer.extend_from_slice(&(image.len() as u64).to_be_bytes());
            footer.resize(64, 0);
            let end = boot.len() - 64;
            boot[end..].copy_from_slice(&footer);
            boot
        };
        fs::write(&boot, partition(0)).unwrap();
        let phone = Phone { by_name: dir.path(), suffix: "_b", locked: false };
        assert_eq!(ran(&["get-verity"], &phone).1, "verity is enabled on slot with suffix _b.\n");
        assert_eq!(ran(&["disable-verity"], &phone).0, 0);
        assert_eq!(fs::read(&boot).unwrap(), partition(1));
    }

    #[test]
    fn what_holds_no_vbmeta_image_fails_and_stays_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone { by_name: dir.path(), suffix: "_a", locked: false };
        let (code, output, errors) = ran(&["get-verity"], &phone);
        assert_eq!((code, output.as_str()), (70, ""), "no partition to look in");
        assert!(errors.ends_with("Error getting whether verity is enabled.\n"), "{errors}");
        fs::write(dir.path().join("boot_a"), vec![0u8; 8192]).unwrap();
        assert_eq!(ran(&["disable-verity"], &phone).0, 70, "a boot partition without a footer");
        let a = dir.path().join("vbmeta_a");
        let other = b"ANDROID!".repeat(64);
        fs::write(&a, &other).unwrap();
        let (code, _, errors) = ran(&["disable-verification"], &phone);
        assert_eq!(code, 70);
        assert!(errors.contains("holds no vbmeta header"), "{errors}");
        assert_eq!(fs::read(&a).unwrap(), other, "nothing written");
    }

    #[test]
    fn without_a_command_it_tells_how_it_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone { by_name: dir.path(), suffix: "_a", locked: false };
        for args in [&[][..], &["--force"], &["disable-dm-verity"]] {
            assert_eq!(ran(args, &phone), (64, String::new(), USAGE.into()), "{args:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// What the recovery's own page uses: the same image, read and turned, without
// the command line.

/// One of the two checks verified boot makes in the slot the phone runs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Verity,
    Verification,
}

impl Switch {
    fn check(self) -> Check {
        match self {
            Switch::Verity => Check::Verity,
            Switch::Verification => Check::Verification,
        }
    }
}

/// The slot the phone runs from, and whether its bootloader is locked.
fn here() -> (String, bool) {
    let suffix = skyward_sys::prop("ro.boot.slot_suffix").unwrap_or_default();
    let locked = skyward_sys::prop("ro.boot.vbmeta.device_state").as_deref() == Some("locked");
    (suffix, locked)
}

fn phone_of(suffix: &str, locked: bool) -> Phone<'_> {
    Phone { by_name: Path::new(skyward_block::BY_NAME), suffix, locked }
}

/// Whether `which` is on in the slot the phone runs from.
pub fn read(which: Switch) -> anyhow::Result<bool> {
    let (suffix, locked) = here();
    is_on(&phone_of(&suffix, locked), which.check()).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Where a vbmeta image's rollback index lies in its header, big-endian.
const ROLLBACK_AT: usize = 112;

/// A copy of the slot's vbmeta image, and the rollback index it carries: a
/// bootloader does not start a slot whose own index is higher than the one
/// written back, so the pair travels together.
pub struct Copy {
    pub bytes: Vec<u8>,
    pub rollback_index: u64,
}

/// The rollback index a vbmeta image's header carries.
pub fn rollback(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes[ROLLBACK_AT..ROLLBACK_AT + 8].try_into().expect("eight bytes"))
}

/// How long the vbmeta image is: its header, then the blocks the header sizes.
fn length(head: &[u8]) -> usize {
    let auth = u64::from_be_bytes(head[12..20].try_into().expect("eight bytes")) as usize;
    let aux = u64::from_be_bytes(head[20..28].try_into().expect("eight bytes")) as usize;
    HEADER_LEN + auth + aux
}

/// The slot's vbmeta image as it lies, read whole.
pub fn dump() -> anyhow::Result<Copy> {
    let (suffix, locked) = here();
    let (path, at) = top_level(&phone_of(&suffix, locked)).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut file = File::open(&path).map_err(|e| about(&path, e))?;
    let head = header(&mut file, &path, at).map_err(|e| anyhow::anyhow!("{e}"))?;
    let whole = length(&head);
    let mut bytes = vec![0u8; whole];
    file.seek(SeekFrom::Start(at)).and_then(|_| file.read_exact(&mut bytes)).map_err(|e| about(&path, e))?;
    Ok(Copy { rollback_index: rollback(&bytes), bytes })
}

/// Writes a copy back over the slot's vbmeta image, refusing one whose
/// rollback index is below the slot's own: that is what makes a bootloader
/// stop starting the slot.
pub fn restore(copy: &Copy) -> anyhow::Result<()> {
    let (suffix, locked) = here();
    let (path, at) = top_level(&phone_of(&suffix, locked)).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut file = OpenOptions::new().read(true).write(true).open(&path).map_err(|e| about(&path, e))?;
    let head = header(&mut file, &path, at).map_err(|e| anyhow::anyhow!("{e}"))?;
    let now = rollback(&head);
    anyhow::ensure!(
        copy.rollback_index >= now,
        "the copy's rollback index {} is below the slot's own {now}: a bootloader does not start a slot rolled back",
        copy.rollback_index
    );
    file.seek(SeekFrom::Start(at)).and_then(|_| file.write_all(&copy.bytes)).and_then(|()| file.sync_all()).map_err(|e| about(&path, e))?;
    Ok(())
}

/// Turns `which` on or off, as the page asked.
pub fn turn(which: Switch, on: bool) -> anyhow::Result<()> {
    let (suffix, locked) = here();
    anyhow::ensure!(
        !locked,
        "the bootloader is locked, and a locked bootloader does not boot a slot whose vbmeta image has been changed"
    );
    set(&phone_of(&suffix, locked), which.check(), on).map_err(|e| anyhow::anyhow!("{e}"))
}
