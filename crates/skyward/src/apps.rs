//! The apps the phone keeps in its system partitions: what each one is, the
//! picture it shows, and the copies kept of them.
//!
//! An app is a folder holding an apk; the recovery reads it after mounting the
//! partition it lives in. A copy keeps the folder as it is, so putting it back
//! is copying it back, and taking one off the phone moves it out of the way
//! rather than losing it: it stays under [`TRASH`] until it is let go of.
//!
//! The work runs on a thread of its own, so the screen goes on while a folder
//! of some megabytes is copied, and tells the page how many are done.

use anyhow::{ensure, Context, Result};
use skyward_ui::app::{AppTask, NewApp};
use std::fs;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

/// Where copies of the apps go, from the top of internal storage.
pub const STORE: &str = "/sdcard/Skyward/apps";

/// Where the apps taken off the phone wait to be put back.
pub const TRASH: &str = "/sdcard/Skyward/apps/trash";

/// The folders under a partition that hold apps.
const KINDS: [&str; 2] = ["app", "priv-app"];

/// The partitions a phone keeps its apps in.
const PARTITIONS: [&str; 4] = ["system", "system_ext", "product", "vendor"];

/// An app the phone keeps.
#[derive(Clone, Debug)]
pub struct App {
    /// What the page names it by, such as `system/priv-app/Settings`.
    pub label: String,
    /// Where it lives, such as `system/priv-app`.
    pub place: String,
    /// Its own folder, in the mounted partition.
    pub dir: PathBuf,
    /// The bytes its folder takes.
    pub bytes: u64,
    /// A copy of it is kept in the store.
    pub kept: bool,
}

impl App {
    /// Where a copy of it is kept, the partition and the folder under it
    /// keeping the places they had on the phone.
    pub fn copy(&self) -> PathBuf {
        Path::new(STORE).join(&self.place).join(self.folder())
    }

    /// Where it waits after being taken off the phone.
    pub fn held(&self) -> PathBuf {
        Path::new(TRASH).join(&self.place).join(self.folder())
    }

    /// The name of its own folder.
    fn folder(&self) -> &str {
        self.label.rsplit('/').next().unwrap_or(&self.label)
    }
}

pub enum Task {
    /// Read the apps, which the page does as it opens.
    Read,
    /// Do this to the apps named, one after another.
    Work { task: AppTask, apps: Vec<String> },
}

pub enum Event {
    /// The apps as read, with the png of each picture, or why they could not
    /// be.
    Read(Result<Vec<NewApp>, String>),
    /// How far the work has come: which, how many done, of how many.
    At(AppTask, usize, usize),
    /// The work ended, having done this many of them, or did not and why.
    Ended(AppTask, usize, Result<(), String>),
}

/// The partitions the phone keeps its apps in, mounted, and where each is
/// mounted: only those that are mounted hold apps the recovery can read.
fn roots() -> Result<Vec<(String, PathBuf)>> {
    let slot = skyward_vab::current_slot()?.context("not an A/B phone")?;
    let shown = skyward_vab::mounted_partitions(slot)?;
    let mut out = Vec::new();
    for base in skyward_vab::slot_bases(slot)? {
        if !PARTITIONS.contains(&base.as_str()) {
            continue;
        }
        let name = format!("{base}{}", slot.suffix());
        if shown.iter().any(|m| m.partition == name) {
            out.push((base.clone(), skyward_vab::mount_point(&base)));
        }
    }
    Ok(out)
}

/// The apps in the partitions mounted at `roots`, each with the bytes it takes
/// and whether a copy of it is kept.
fn scan(roots: &[(String, PathBuf)]) -> Result<Vec<App>> {
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
                let Some(name) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
                let place = format!("{partition}/{kind}");
                let label = format!("{place}/{name}");
                let kept = Path::new(STORE).join(&place).join(&name).is_dir();
                let bytes = measure(&dir);
                out.push(App { label, place, dir, bytes, kept });
            }
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(out)
}

/// The bytes under `dir`, counted.
fn measure(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    let mut total = 0;
    for entry in entries.flatten() {
        match entry.metadata() {
            Ok(m) if m.is_dir() => total += measure(&entry.path()),
            Ok(m) => total += m.len(),
            Err(_) => {}
        }
    }
    total
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

/// The picture an apk shows: the largest `ic_launcher` png in it, which is the
/// name an app's own icon is stored under.
fn icon_in(app: &App) -> Option<Vec<u8>> {
    let apk = apk_in(&app.dir).ok()?;
    let file = fs::File::open(&apk).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
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
    best.map(|(_, bytes)| bytes)
}

/// The apps as the page shows them, each with the png of its picture.
fn read(roots: &[(String, PathBuf)]) -> Result<Vec<NewApp>> {
    let apps = scan(roots)?;
    Ok(apps
        .into_iter()
        .map(|a| NewApp {
            icon: icon_in(&a),
            label: a.label,
            place: a.place,
            bytes: a.bytes,
            kept: a.kept,
        })
        .collect())
}

/// Copy the app's folder into the store, as it is.
fn back_up(app: &App) -> Result<()> {
    copy_tree(&app.dir, &app.copy())
}

/// Put the app's folder back where it was, from the store, where a copy of it
/// is kept.
fn put_back(app: &App) -> Result<()> {
    let from = app.copy();
    ensure!(from.is_dir(), "{} holds no copy of {} to put back", STORE, app.label);
    copy_tree(&from, &app.dir)
}

/// Take the app off the phone, keeping it in the trash to be put back.
fn remove(app: &App) -> Result<()> {
    copy_tree(&app.dir, &app.held())?;
    fs::remove_dir_all(&app.dir).with_context(|| format!("take away {}", app.dir.display()))
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

/// Do `task` to the apps named, telling how many are done after each.
fn work(task: AppTask, apps: &[&App], tell: &mut impl FnMut(Event)) -> (usize, Result<()>) {
    let total = apps.len();
    let mut done = 0;
    for app in apps {
        let one = match task {
            AppTask::BackUp => back_up(app),
            AppTask::Restore => put_back(app),
            AppTask::Remove => remove(app),
        };
        if let Err(e) = one {
            return (done, Err(e));
        }
        done += 1;
        tell(Event::At(task, done, total));
    }
    (done, Ok(()))
}

/// The work on the apps, from a thread of its own.
pub struct Worker {
    /// The thread ends once it has no more tasks to take: this end goes
    /// before the thread is waited for, being the field before it.
    tasks: Sender<Task>,
    /// The tasks sent and not yet ended.
    pending: usize,
    worker: skyward_sys::worker::Worker<Event>,
}

impl Worker {
    pub fn start() -> Result<Worker> {
        let (tasks, todo) = mpsc::channel::<Task>();
        let worker = skyward_sys::worker::Worker::start("apps", move |tell| {
            for task in todo {
                let list = || read(&roots().unwrap_or_default()).map_err(|e| format!("{e:#}"));
                match task {
                    Task::Read => {}
                    Task::Work { task, apps } => {
                        let (done, ended) = match roots().and_then(|roots| scan(&roots)) {
                            Ok(held) => {
                                let picked: Vec<&App> = apps.iter().filter_map(|l| held.iter().find(|a| &a.label == l)).collect();
                                work(task, &picked, &mut |event| {
                                    tell.tell(event);
                                })
                            }
                            Err(e) => (0, Err(e)),
                        };
                        tell.tell(Event::Ended(task, done, ended.map_err(|e| format!("{e:#}"))));
                    }
                }
                tell.tell(Event::Read(list()));
            }
        })?;
        Ok(Worker { tasks, pending: 0, worker })
    }

    pub fn send(&mut self, task: Task) {
        self.tasks.send(task).expect("the apps' worker takes tasks while the screen holds it");
        self.pending += 1;
    }

    /// Readable while events wait.
    pub fn fd(&self) -> RawFd {
        self.worker.fd()
    }

    /// Whether a task is under way or waiting.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// The events since the last call.
    pub fn events(&mut self) -> Vec<Event> {
        let events = self.worker.events();
        self.pending -= events.iter().filter(|e| matches!(e, Event::Read(_))).count();
        events
    }
}