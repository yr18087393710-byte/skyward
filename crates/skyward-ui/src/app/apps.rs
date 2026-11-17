//! The page of the apps the phone keeps in its system partitions: what each
//! one is, the copy kept of it, and the work on the ones ticked.

use super::*;

/// How wide the square an app's picture is drawn in is, in dp.
pub const ICON_DP: f32 = 40.0;

/// An app as the runtime read it, and the png of the picture it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct NewApp {
    /// What the page names it by, such as `system/priv-app/Settings`.
    pub label: String,
    /// Where it lives, such as `system/priv-app`.
    pub place: String,
    /// The bytes its folder takes.
    pub bytes: u64,
    /// Whether a copy of it is kept in the store.
    pub kept: bool,
    /// The png of the picture it shows, where its apk holds one.
    pub icon: Option<Vec<u8>>,
}

/// An app, as the page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRow {
    pub label: String,
    pub place: String,
    pub bytes: u64,
    /// A copy of it is kept in the store, to be put back.
    pub kept: bool,
    pub icon: Option<crate::appicon::Picture>,
}

impl AppRow {
    /// The name the row shows: the folder's own name.
    pub fn name(&self) -> &str {
        self.label.rsplit('/').next().unwrap_or(&self.label)
    }
}

/// The apps ticked on the page, in the order it lists them.
pub fn ticked(state: &State) -> Vec<String> {
    match &state.apps {
        Some(Ok(rows)) => rows.iter().filter(|a| state.chosen_apps.contains(&a.label)).map(|a| a.label.clone()).collect(),
        _ => Vec::new(),
    }
}

/// Whether a copy of one of the apps ticked is kept in the store.
pub fn any_kept(state: &State) -> bool {
    match &state.apps {
        Some(Ok(rows)) => rows.iter().any(|a| a.kept && state.chosen_apps.contains(&a.label)),
        _ => false,
    }
}

/// What the page does to the apps ticked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppTask {
    /// Copy them into the store.
    BackUp,
    /// Put them back from the store.
    Restore,
    /// Take them off the phone, keeping them in the trash.
    Remove,
}

impl AppTask {
    /// What the page says is being done.
    pub fn doing(self, l: Lang) -> &'static str {
        l.pick(match self {
            AppTask::BackUp => ["正在备份", "正在備份", "Backing up"],
            AppTask::Restore => ["正在恢复", "正在還原", "Restoring"],
            AppTask::Remove => ["正在移到回收站", "正在移到回收筒", "Moving to the trash"],
        })
    }

    /// What the page says once it is done.
    pub fn did(self, l: Lang) -> &'static str {
        l.pick(match self {
            AppTask::BackUp => ["已备份", "已備份", "Backed up"],
            AppTask::Restore => ["已恢复", "已還原", "Restored"],
            AppTask::Remove => ["已移到回收站", "已移到回收筒", "Moved to the trash"],
        })
    }
}

/// Work on the apps under way: which task, how many of them are done, and how
/// many there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppsWork {
    pub task: AppTask,
    pub done: usize,
    pub total: usize,
}

impl AppsWork {
    /// How far it has come, 0 to 1.
    pub fn level(&self) -> f32 {
        match self.total {
            0 => 0.0,
            total => self.done as f32 / total as f32,
        }
    }
}

impl App {
    /// The apps, as the runtime read them, or why it could not; the pictures
    /// they show are read into the square the rows draw them in. Work on them
    /// that was under way has ended.
    pub fn set_apps(&mut self, apps: Result<Vec<NewApp>, String>) {
        let px = self.theme().m.dp(ICON_DP).round().max(1.0) as u32;
        let rows = apps.map(|rows| {
            rows.into_iter()
                .map(|a| AppRow {
                    label: a.label,
                    place: a.place,
                    bytes: a.bytes,
                    kept: a.kept,
                    icon: a.icon.as_deref().and_then(|png| crate::appicon::Picture::png(png, px).ok()),
                })
                .collect::<Vec<AppRow>>()
        });
        // Of the apps ticked, those the phone no longer holds are let go.
        if let Ok(rows) = &rows {
            let held: std::collections::BTreeSet<&str> = rows.iter().map(|a| a.label.as_str()).collect();
            let chosen = self.state.chosen_apps.clone();
            for label in chosen {
                if !held.contains(label.as_str()) {
                    self.state.chosen_apps.remove(&label);
                }
            }
        }
        self.state.apps = Some(rows);
        self.state.apps_work = None;
        self.refresh();
    }

    /// How far the work on the apps has come.
    pub fn apps_at(&mut self, task: AppTask, done: usize, total: usize) {
        let work = AppsWork { task, done, total };
        if self.state.apps_work != Some(work) {
            self.state.apps_work = Some(work);
            self.refresh();
        }
    }

    /// How the work on the apps ended: what it did to how many of them, or why
    /// it did not.
    pub fn apps_ended(&mut self, task: AppTask, done: usize, ended: Result<(), String>, now: u64) {
        let l = self.state.lang;
        let did = task.did(l);
        let said = match ended {
            Ok(()) => tr!(l, "{did} {done} 个应用", "{did} {done} 個應用程式", "{did} {done} apps"),
            Err(why) => match task {
                AppTask::BackUp => tr!(l, "备份失败：{why}", "備份失敗：{why}", "Couldn't back up: {why}"),
                AppTask::Restore => tr!(l, "恢复失败：{why}", "還原失敗：{why}", "Couldn't restore: {why}"),
                AppTask::Remove => tr!(l, "移入回收站失败：{why}", "移入回收筒失敗：{why}", "Couldn't move to the trash: {why}"),
            },
        };
        self.state.apps_work = None;
        self.state.asking_apps = false;
        self.say(&said, now);
        self.refresh();
    }
}