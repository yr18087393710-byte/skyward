//! What a control does once it is tapped, typed on or swiped.

use super::*;

impl App {
    /// Read `url`, or the address the field holds where `None` is given,
    /// into the browser; a page reached from a link takes the field's place
    /// in the pages seen, and one typed past a page already seen gives up
    /// what was ahead.
    fn browse_to(&mut self, url: Option<String>, now: u64) -> Vec<Effect> {
        let l = self.state.lang;
        if self.state.browsing.loading() {
            self.say(l.pick(["正在打开网页", "正在開啟網頁", "A page is opening"]), now);
            return Vec::new();
        }
        let addressed = match url {
            Some(url) => Some(url),
            None => skyward_net::page::address(&self.state.browse_url),
        };
        let Some(url) = addressed else {
            self.say(l.pick(["请输入网页地址", "請輸入網頁位址", "Type a web address"]), now);
            return Vec::new();
        };
        if self.state.trail.get(self.state.trail_at) != Some(&url) {
            // Going somewhere else from a page gone back to gives up the
            // pages that were ahead of it.
            if self.state.trail.len() > self.state.trail_at + 1 {
                self.state.trail.truncate(self.state.trail_at + 1);
            }
            self.state.trail.push(url.clone());
            self.state.trail_at = self.state.trail.len() - 1;
        }
        self.state.browse_url = url.clone();
        self.state.browsing = Browsing::Loading;
        self.refresh();
        vec![Effect::Browse(url)]
    }

    /// Carry out a tapped control.
    pub(super) fn act(&mut self, action: Action, now: u64) -> Vec<Effect> {
        let l = self.state.lang;
        match action {
            Action::Open(s) => {
                // The served folder is chosen once: opening the page it is
                // served on ends the choosing, however it was got to.
                if s == Screen::Serve {
                    self.state.picking = false;
                }
                if s == Screen::Search && self.screen() == Screen::Home {
                    // The bar moves up from where it stands unfolded: a
                    // page easing there for the keys gets there at once.
                    if let Some((rest, _)) = self.seek.take() {
                        self.top_mut().1 = rest;
                    }
                    self.start_morph(true, now);
                    self.push(s);
                } else {
                    self.open(s);
                }
                return self.arrive_at(s);
            }
            Action::OpenData => return self.open_data(),
            // The order the file manager shows entries in: the key cycles, the
            // side turns, and the folders stay on top or mix in.
            Action::CycleSort => {
                self.state.sort = match self.state.sort {
                    SortKey::Name => SortKey::Size,
                    SortKey::Size => SortKey::Modified,
                    SortKey::Modified => SortKey::Type,
                    SortKey::Type => SortKey::Name,
                };
                self.resort_files();
                return vec![Effect::KeepSort(self.state.sort, self.state.sort_desc, self.state.folders_first)];
            }
            Action::ToggleSortOrder => {
                self.state.sort_desc = !self.state.sort_desc;
                self.resort_files();
                return vec![Effect::KeepSort(self.state.sort, self.state.sort_desc, self.state.folders_first)];
            }
            Action::ToggleFoldersFirst => {
                self.state.folders_first = !self.state.folders_first;
                self.resort_files();
                return vec![Effect::KeepSort(self.state.sort, self.state.sort_desc, self.state.folders_first)];
            }
            // A row turns the check the page shows; the swipe below writes both.
            Action::Avb(which) => {
                let (mut verity, mut verification) = match (self.state.avb_pending, &self.state.avb) {
                    (Some(pending), _) => pending,
                    (None, Some(Ok(now))) => *now,
                    _ => (true, true),
                };
                if which {
                    verification = !verification;
                } else {
                    verity = !verity;
                }
                self.state.avb_pending = Some((verity, verification));
                self.refresh();
            }
            Action::FetchStart => {
                let url = self.state.fetch_url.trim().to_string();
                if url.is_empty() {
                    self.say(self.state.lang.pick(["请先输入网址", "請先輸入網址", "Type an address first"]), now);
                } else if self.fetching() {
                    self.say(self.state.lang.pick(["已有下载在进行", "已有下載在進行", "A download is under way"]), now);
                } else {
                    self.state.fetch = Fetch::Running { done: 0, total: None };
                    return vec![Effect::Fetch(url)];
                }
            }
            Action::FetchStop => {
                if self.fetching() {
                    return vec![Effect::StopFetch];
                }
            }
            // The browser reads the address it is given, and the links it
            // finds; the pages read are kept in the order they were seen, so
            // that back and forward walk them.
            Action::BrowseGo => return self.browse_to(None, now),
            Action::BrowseStop => {
                if self.state.browsing.loading() {
                    return vec![Effect::StopBrowse];
                }
            }
            Action::BrowseLink(i) => {
                let Some(page) = self.state.browsing.page() else { return Vec::new() };
                let Some(link) = page.links.get(i).map(|l| l.url.clone()) else { return Vec::new() };
                return self.browse_to(Some(link), now);
            }
            Action::BrowseBack => {
                if self.state.trail_at > 0 {
                    self.state.trail_at -= 1;
                    let url = self.state.trail[self.state.trail_at].clone();
                    return self.browse_to(Some(url), now);
                }
            }
            Action::BrowseForward => {
                if self.state.trail_at + 1 < self.state.trail.len() {
                    self.state.trail_at += 1;
                    let url = self.state.trail[self.state.trail_at].clone();
                    return self.browse_to(Some(url), now);
                }
            }
            Action::BrowseDownload => {
                let url = match self.state.browsing.page().map(|p| p.url.clone()).or_else(|| {
                    let typed = self.state.browse_url.trim();
                    match typed.is_empty() {
                        true => None,
                        false => skyward_net::page::address(typed),
                    }
                }) {
                    Some(url) => url,
                    None => {
                        self.say(l.pick(["请先打开一个网页", "請先開啟一個網頁", "Open a page first"]), now);
                        return Vec::new();
                    }
                };
                self.state.fetch_url = url;
                self.state.fetch = Fetch::Idle;
                self.open(Screen::Fetch);
                return Vec::new();
            }
            // An address too long to type can be put in a file from a computer
            // and taken from there with one tap.
            Action::FetchFromFile => {
                let at = "/sdcard/Download/link.txt";
                match std::fs::read_to_string(at) {
                    Ok(text) => {
                        let url = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("").to_string();
                        let said = match url.is_empty() {
                            true => self.state.lang.pick(["文件里没有链接", "檔案裡沒有連結", "The file holds no link"]).to_string(),
                            false => {
                                self.state.fetch_url = url;
                                tr!(self.state.lang, "已从文件读入链接", "已從檔案讀入連結", "The link was taken from the file").to_string()
                            }
                        };
                        self.say(&said, now);
                    }
                    Err(e) => {
                        let said = format!("{at}: {e}");
                        self.say(&said, now);
                    }
                }
                self.dirty = true;
            }
            // The folder served is the file manager's own: the switch serves
            // it or stops serving it, and changing it opens the file manager,
            // which the page returns to.
            Action::ServeToggle => {
                let on = !matches!(self.state.serving, Serving::Running { .. });
                return vec![Effect::Serve(on)];
            }
            Action::ServePick => {
                self.state.picking = true;
                return self.act(Action::Open(Screen::Files), now);
            }
            // The folder shown is the one that will be served: the bar at the
            // foot of the file manager takes it.
            Action::ServeUseHere => {
                self.state.picking = false;
                let mut fx = self.act(Action::Open(Screen::Serve), now);
                fx.insert(0, Effect::Serve(true));
                return fx;
            }
            Action::Skip => {
                self.leave(true);
                if self.stack.len() > 1 {
                    self.stack.pop();
                } else {
                    *self.top_mut() = (Screen::Home, 0.0, Focus::Unset);
                }
                self.settle_page();
                self.refresh();
            }
            // The card asking to delete is what the page shows: back puts it
            // away before it leaves the page.
            Action::Back if self.state.asking_delete || self.state.asking_many.is_some() => {
                self.state.asking_delete = false;
                self.state.asking_many = None;
                self.state.confirm = 0.0;
                self.refresh();
            }
            Action::Back | Action::CloseSearch => return self.back(now),
            Action::Theme { dark } => {
                if self.state.dark != dark {
                    self.state.dark = dark;
                    self.refresh();
                }
            }
            // The switch moves at once; the runtime then tells how the WLAN
            // came up, or went off.
            Action::Toggle(Switch::Wlan) => {
                let w = self.state.wlan.as_mut().expect("the WLAN switch shows on a phone whose WLAN is brought up");
                let on = matches!(w.power, Power::Off | Power::Failed(_));
                *w = Wlan { power: if on { Power::Starting } else { Power::Off }, ..Wlan::default() };
                self.refresh();
                return vec![Effect::Wlan(on)];
            }
            Action::Toggle(switch) => {
                // Keeping the screen lock is a risk taken on, and it is said
                // every time it is turned on, not once: the switch stays off
                // until the slider is taken across. A second tap while the
                // risk is shown puts it away again.
                if switch == Switch::Remember {
                    if self.state.asking_remember {
                        self.state.asking_remember = false;
                        self.refresh();
                        return Vec::new();
                    }
                    if !self.state.remember {
                        self.state.asking_remember = true;
                        self.refresh();
                        return Vec::new();
                    }
                    self.state.remember = false;
                    self.refresh();
                    return vec![Effect::Remember(false)];
                }
                let on = match switch {
                    Switch::Vibrate => &mut self.state.vibrate,
                    Switch::FileTransfer => &mut self.state.file_transfer,
                    Switch::Compress => &mut self.state.compress,
                    Switch::Verify => &mut self.state.verify,
                    Switch::Signature => &mut self.state.check_signature,
                    Switch::KeepRecovery => &mut self.state.keep_recovery,
                    Switch::ShowPassword => &mut self.state.show_password,
                    Switch::Remember => &mut self.state.remember,
                    Switch::WlanAuto => &mut self.state.wlan_auto,
                    Switch::WlanKeep => &mut self.state.wlan_keep,
                    Switch::Wlan => unreachable!("the WLAN switch is turned above"),
                };
                *on = !*on;
                let now_on = *on;
                self.refresh();
                if switch == Switch::FileTransfer {
                    return vec![Effect::FileTransfer(now_on)];
                }
                if switch == Switch::Remember {
                    return vec![Effect::Remember(now_on)];
                }
                if switch == Switch::WlanAuto {
                    return vec![Effect::WlanAuto(now_on)];
                }
                if switch == Switch::WlanKeep {
                    return vec![Effect::WlanKeep(now_on)];
                }
            }
            Action::MountToggle(i) => {
                if self.state.mount_failure.take().is_some() {
                    self.refresh();
                }
                let row = &self.state.mounts.as_ref().expect("a partition toggled is one the page lists")[i];
                let base = row.base.clone();
                return vec![if row.mounted { Effect::Unmount(base) } else { Effect::Mount(base) }];
            }
            Action::RebootTarget(i) => {
                self.state.reboot = i;
                self.refresh();
            }
            // Everything takes in every part, so choosing it leaves the
            // parts out, and choosing a part on its own leaves it out.
            Action::WipeItem(item) => {
                let k = WIPES.iter().position(|&w| w == item).expect("the page offers only what it lists");
                let on = !self.state.wipe[k];
                match (on, item) {
                    (true, Wipe::All) => {
                        self.state.wipe = [false; WIPES.len()];
                        self.state.wipe[k] = true;
                    }
                    (true, _) => {
                        self.state.wipe[0] = false;
                        self.state.wipe[k] = true;
                    }
                    (false, _) => self.state.wipe[k] = false,
                }
                self.refresh();
            }
            Action::Confirm(_) => self.say(l.pick(["请按住圆形按钮向右滑动", "請按住圓形按鈕向右滑動", "Hold the round button and slide it right"]), now),
            Action::Slider(_) | Action::Pattern => {}
            Action::Sideload => match self.state.install {
                Install::Idle | Install::Finished { .. } => {
                    self.begin_install(Install::Starting, Source::Computer, None);
                    if self.screen() == Screen::Installing {
                        self.refresh();
                    } else {
                        self.open(Screen::Installing);
                    }
                    return vec![Effect::Sideload];
                }
                _ => self.say(l.pick(INSTALL_UNFINISHED), now),
            },
            Action::FlashPartition(k) => {
                if let Install::Choosing(choice) = &mut self.state.install {
                    choice.partition = k;
                    self.refresh();
                }
            }
            Action::FlashSlots(pick) => {
                if let Install::Choosing(choice) = &mut self.state.install {
                    choice.pick = pick;
                    self.refresh();
                }
            }
            Action::SystemSize(k) => {
                if let Install::SystemChoosing(choice) = &mut self.state.install {
                    choice.size = k;
                    self.refresh();
                }
            }
            Action::PickSystem(who) => {
                if self.state.systems_work.is_none() {
                    self.state.system_pick = Some(who);
                    self.state.asking_delete = false;
                    self.state.confirm = 0.0;
                    self.refresh();
                }
            }
            Action::RebootTo(target) => return vec![Effect::Reboot(target)],
            Action::EnterRecovery => return vec![Effect::EnterRecovery],
            Action::Dismiss => {
                self.stack = vec![(Screen::Home, 0.0, Focus::Unset)];
                self.settle_page();
                self.refresh();
                return vec![Effect::Dismiss];
            }
            Action::PickEntry(op) => {
                let path = self.entry_path();
                self.state.clip = Some(Clip { op, path });
                return self.back(now);
            }
            // A folder cannot go inside itself, and an entry moved to the
            // folder it is in stays put: the entry stays picked for another
            // folder.
            Action::Paste => {
                let clip = self.state.clip.clone().expect("only a picked entry is pasted");
                let here = self.state.folder_here(self.screen());
                let (path, folder) = (std::path::Path::new(&clip.path), std::path::Path::new(&here));
                let name = path.file_name().map_or_else(|| clip.path.clone(), |n| n.to_string_lossy().into_owned());
                if folder.starts_with(path) {
                    self.say(&tr!(l, "无法将\"{name}\"放入其自身中", "無法將「{name}」放入其本身中", "“{name}” can't go inside itself"), now);
                } else if clip.op == FileOp::Move && path.parent() == Some(folder) {
                    self.say(&tr!(l, "\"{name}\"已在该文件夹中", "「{name}」已在這個資料夾中", "“{name}” is already in this folder"), now);
                } else {
                    return vec![Effect::Paste { op: clip.op, path: clip.path, into: here, clash: None }];
                }
            }
            Action::Unpick => {
                self.state.clip = None;
                self.refresh();
            }
            Action::RenameEntry => {
                self.state.new_name = self.state.file.as_ref().expect("the file page shows an entry").name.clone();
                self.state.layer = Layer::Letters;
                self.open(Screen::Rename);
            }
            Action::EditEntry => return vec![Effect::OpenText(self.entry_path())],
            Action::InstallEntry => {
                let size = self.state.file.as_ref().expect("the file page shows an entry").size;
                return self.install_opened(self.entry_path(), size, now);
            }
            Action::SaveText => {
                let ed = self.state.editor.as_ref().expect("the editor page shows a file");
                return vec![Effect::SaveText { path: ed.path.clone(), text: ed.text() }];
            }
            Action::OpenPermissions => self.open(Screen::Permissions),
            Action::ModeBit(bit) => {
                let mode = self.state.file.as_ref().expect("the permissions page shows an entry").mode;
                return vec![Effect::SetMode { path: self.entry_path(), mode: (mode ^ bit) & 0o7777 }];
            }
            Action::AskDelete => {
                self.state.asking_delete = true;
                self.state.confirm = 0.0;
                self.refresh();
            }
            // Choosing several entries of the folder shown: a tick beside each,
            // and the bar at the foot carrying what can be done with them.
            Action::ChooseMany(on) => {
                self.state.choosing = on;
                self.state.chosen.clear();
                self.refresh();
            }
            Action::ChooseEntryAt(i) => {
                let folder = &self.state.files;
                if let Ok(entries) = &folder.entries
                    && let Some(e) = entries.get(i)
                {
                    let name = e.name.clone();
                    if !self.state.chosen.remove(&name) {
                        self.state.chosen.insert(name);
                    }
                    self.refresh();
                }
            }
            Action::ChooseAll => {
                let all: Vec<String> = self.state.files.entries.as_ref().map(|e| e.iter().map(|r| r.name.clone()).collect()).unwrap_or_default();
                if self.state.chosen.len() == all.len() {
                    self.state.chosen.clear();
                } else {
                    self.state.chosen = all.into_iter().collect();
                }
                self.refresh();
            }
            Action::PasteChosen(op) => {
                let into = self.state.folder_here(self.screen());
                let paths: Vec<String> = self.state.chosen.iter().map(|n| std::path::Path::new(&into).join(n).to_string_lossy().into_owned()).collect();
                if paths.is_empty() {
                    return Vec::new();
                }
                self.state.choosing = false;
                self.state.chosen.clear();
                self.refresh();
                return vec![Effect::PasteMany { op, paths, into, clash: None }];
            }
            // Deleting several is as destructive as deleting one, and asks the
            // same way: a swipe confirms, and nothing goes before it.
            Action::AskDeleteChosen => {
                let into = self.state.files.path.clone();
                let paths: Vec<String> = self.state.chosen.iter().map(|n| std::path::Path::new(&into).join(n).to_string_lossy().into_owned()).collect();
                if paths.is_empty() {
                    return Vec::new();
                }
                self.state.asking_many = Some(paths);
                self.state.confirm = 0.0;
                self.refresh();
            }
            Action::CancelDelete => {
                self.state.asking_delete = false;
                self.state.asking_many = None;
                self.state.confirm = 0.0;
                self.refresh();
            }
            Action::Resolve(choice) => {
                let (clip, into, _) = self.state.clash.take().expect("a choice answers a clash");
                self.leave(true);
                self.stack.pop();
                self.settle_page();
                self.refresh();
                // Skipping the one entry picked leaves nothing to do.
                if choice == ClashChoice::Skip {
                    self.state.clip = None;
                    return Vec::new();
                }
                return vec![Effect::Paste { op: clip.op, path: clip.path, into, clash: Some(choice) }];
            }
            Action::StopJob => {
                let job = self.state.job.as_mut().expect("only work under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopJob];
                }
            }
            Action::StopRelabel => {
                let job = self.state.relabel.as_mut().expect("only work under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopRelabel];
                }
            }
            Action::OpenFilesystem(k) => {
                self.state.fs_chosen = k;
                self.state.fs_action = FsAction::Repair;
                self.open(Screen::Filesystem);
            }
            Action::ChooseFs(choice) => {
                let row = self.fs_row().expect("the page of a partition shows one that was read");
                let action = match choice {
                    FsChoice::Repair => FsAction::Repair,
                    FsChoice::Remake(k) => FsAction::Remake(row.others[k].clone()),
                };
                self.state.fs_action = action;
                self.refresh();
            }
            Action::ScreenOff(k) => {
                self.state.screen_off = k;
                self.refresh();
            }
            Action::TimeZone(k) => {
                self.state.time_zone = k;
                self.refresh();
            }
            Action::Language(l) => {
                self.state.lang = l;
                self.state.language_chosen = true;
                self.refresh();
            }
            Action::BackupPart(k) => {
                self.state.backup[k] = !self.state.backup[k];
                self.refresh();
            }
            Action::BackupTo(place) => {
                self.state.backup_to = place;
                self.refresh();
            }
            // All a backup holds is ticked to restore at first.
            Action::OpenBackup(i) => {
                let entry = self.state.backups[i].clone();
                let parts = entry.held.as_ref().map_or([false; 4], |h| h.parts);
                self.state.restore = Some((entry, parts));
                self.state.asking_delete = false;
                self.open(Screen::RestoreBackup);
            }
            Action::RestorePart(k) => {
                let (_, parts) = self.state.restore.as_mut().expect("the restore page shows a backup");
                parts[k] = !parts[k];
                self.refresh();
            }
            Action::StopBackup => {
                let job = self.state.backup_job.as_mut().expect("only work under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopBackup];
                }
            }
            Action::LogPart(k) => {
                self.state.logs[k] = !self.state.logs[k];
                self.refresh();
            }
            Action::LogsTo(k) => {
                self.state.logs_to = k;
                self.refresh();
            }
            Action::ExportLogs => {
                let ([recovery, kernel], storage) = (self.state.logs, self.state.logs_to == 1);
                if !recovery && !kernel {
                    self.say(l.pick(["请选择要导出的日志", "請選擇要匯出的記錄", "Choose the logs to export"]), now);
                } else if storage && !self.state.data.storage_open() {
                    self.say(l.pick(DECRYPT_FIRST), now);
                } else {
                    return vec![Effect::ExportLogs { recovery, kernel, storage }];
                }
            }
            Action::PickSlot(slot) => {
                self.state.next_slot = Some(slot);
                self.refresh();
            }
            Action::Type(cap) if self.screen() == Screen::Terminal => return self.term_typed(cap),
            Action::Type(cap) if self.screen() == Screen::Editor => self.edit_typed(cap),
            Action::Type(cap) => return self.type_key(cap, now),
            Action::TermKey(key) if self.screen() == Screen::Editor => {
                let ed = self.state.editor.as_mut().expect("the editor page shows a file");
                match key {
                    TermKey::Tab => ed.insert('\t'),
                    TermKey::Left => ed.left(),
                    TermKey::Up => ed.up(),
                    TermKey::Down => ed.down(),
                    TermKey::Right => ed.right(),
                    TermKey::Esc | TermKey::Ctrl => unreachable!("the editor's row has no {key:?}"),
                }
                self.edits += 1;
                self.refresh_text();
            }
            Action::TermKey(key) => {
                let Some(term) = self.term.as_mut() else { return Vec::new() };
                let bytes = term.key(key);
                let running = term.running;
                self.refresh();
                if running && !bytes.is_empty() {
                    return vec![Effect::TermInput(bytes)];
                }
            }
            Action::ClearSearch => {
                // The button empties the field the page types into, which on
                // the search page is the search and on the download and
                // browser pages the address.
                match self.text_field() {
                    Some(field) => field.clear(),
                    None => self.state.search.clear(),
                }
                self.edits += 1;
                self.refresh();
            }
            Action::OpenEntry(i) => {
                let entry = self.state.files.entry(i).clone();
                match entry.kind {
                    FileKind::Folder { .. } | FileKind::Link { folder: true, .. } => {
                        let scroll = self.top().1;
                        self.state.files.leave(scroll);
                        return vec![Effect::ReadDir(self.state.files.inside(&entry.name))];
                    }
                    FileKind::File if matches!(files::file_type(&entry.name), FileType::Text | FileType::Code) => {
                        return vec![Effect::OpenText(self.state.files.inside(&entry.name))];
                    }
                    _ if installs(&entry) => {
                        let path = self.state.files.inside(&entry.name);
                        return self.install_opened(path, entry.size, now);
                    }
                    FileKind::File | FileKind::Link { folder: false, .. } | FileKind::Special => {
                        self.state.file = Some(entry);
                        self.open(Screen::FileInfo);
                    }
                }
            }
            // A shell already running goes to the folder; otherwise one starts
            // there.
            Action::TerminalHere => {
                let folder = self.state.files.path.clone();
                let running = self.term.as_ref().is_some_and(|term| term.running);
                self.open(Screen::Terminal);
                self.state.layer = Layer::Letters;
                if running {
                    return vec![Effect::ShellTo(folder)];
                }
                self.shell_folder = folder.clone();
                return vec![Effect::StartShell(folder)];
            }
            Action::ChooseEntry(i) => {
                let screen = self.screen();
                // A tap that lands as the page turns is nobody's mistake: the
                // place's page is the only one that chooses, and one that is
                // not showing has nothing to choose.
                let Screen::Choose(place) = screen else { return Vec::new() };
                let folder = self.state.folder(screen);
                let Ok(entries) = &folder.entries else { return Vec::new() };
                let Some(entry) = entries.get(i).cloned() else { return Vec::new() };
                let path = folder.inside(&entry.name);
                match entry.kind {
                    FileKind::Folder { .. } | FileKind::Link { folder: true, .. } => {
                        let scroll = self.top().1;
                        self.state.folder_mut(screen).leave(scroll);
                        return vec![Effect::ReadPlace(place, path)];
                    }
                    FileKind::File | FileKind::Link { folder: false, .. } => return self.install_file(place, path, entry.size, now),
                    FileKind::Special => unreachable!("a place lists no special files"),
                }
            }
            Action::OpenAncestor(depth) => {
                let screen = self.screen();
                if let Some(path) = self.state.folder(screen).ancestor(depth) {
                    return vec![match screen {
                        Screen::Choose(place) => Effect::ReadPlace(place, path),
                        _ => Effect::ReadDir(path),
                    }];
                }
            }
            Action::JoinNetwork(i) => {
                let w = self.state.wlan.as_ref().expect("networks show on a phone whose WLAN is brought up");
                // A scan may have changed the list since the page was drawn.
                let Some(network) = w.networks.get(i).cloned() else { return Vec::new() };
                if w.joined.as_ref().is_some_and(|j| j.ssid == network.ssid) {
                    return self.act(Action::OpenJoined, now);
                }
                match network.security {
                    Security::Wep => self.say(l.pick(["暂不支持 WEP 加密的网络", "尚不支援 WEP 加密的網路", "WEP networks aren't supported"]), now),
                    Security::Enterprise => {
                        self.say(l.pick(["暂不支持需要账号登录的网络", "尚不支援需要帳號登入的網路", "Networks that need an account sign-in aren't supported"]), now)
                    }
                    s if s.password() => self.ask_password(network),
                    _ => {
                        return vec![Effect::JoinWlan {
                            ssid: network.ssid,
                            name: network.name,
                            security: network.security,
                            password: Secret::default(),
                        }]
                    }
                }
            }
            Action::NetworkDetails(i) => {
                let w = self.state.wlan.as_ref().expect("networks show on a phone whose WLAN is brought up");
                let Some(network) = w.networks.get(i).cloned() else { return Vec::new() };
                self.state.details = Some(network);
                self.open(Screen::WlanNetwork);
            }
            Action::OpenJoined => {
                let w = self.state.wlan.as_ref().expect("a network is chosen on a phone whose WLAN is brought up");
                let Some(j) = w.joined.clone() else { return Vec::new() };
                // As found, where it still is; as it was chosen otherwise.
                let found = w.networks.iter().find(|n| n.ssid == j.ssid).cloned();
                let network = found.unwrap_or(Network {
                    ssid: j.ssid,
                    name: j.name,
                    signal: 0,
                    frequency: 0,
                    bands: Default::default(),
                    security: j.security,
                });
                if j.stage == Joining::WrongPassword {
                    self.ask_password(network);
                } else {
                    self.state.details = Some(network);
                    self.open(Screen::WlanNetwork);
                }
            }
            Action::LeaveNetwork => {
                let mut fx = self.back(now);
                fx.push(Effect::LeaveWlan);
                return fx;
            }
            Action::Rescan => return vec![Effect::ScanWlan],
            Action::AddNetwork => {
                self.state.adding = Some(Adding { name: String::new(), security: Security::Open, field: Field::Name, choosing: false });
                self.state.password.clear();
                self.state.show_password = false;
                self.state.layer = Layer::Letters;
                self.open(Screen::WlanAdd);
            }
            Action::TypeInto(field) => {
                let a = self.state.adding.as_mut().expect("the fields are on the page that adds a network");
                if a.field != field {
                    a.field = field;
                    self.refresh();
                }
            }
            Action::ChooseSecurity => {
                self.state.adding.as_mut().expect("the protection is on the page that adds a network").choosing = true;
                // The page draws the row held down while its list is open.
                self.row_press = None;
                self.row_art = None;
                self.refresh();
            }
            Action::PickSecurity(security) => {
                let a = self.state.adding.as_mut().expect("the protections open on the page that adds a network");
                a.security = security;
                a.choosing = false;
                // With no password to type, the keyboard types the name.
                if !security.password() {
                    a.field = Field::Name;
                }
                self.refresh();
            }
            Action::CloseChoices => {
                self.state.adding.as_mut().expect("the protections open on the page that adds a network").choosing = false;
                self.refresh();
            }
            // Asking takes a network, which the WLAN page joins.
            Action::CheckUpdate => {
                if !self.state.online() {
                    self.open(Screen::Wlan);
                    return self.arrive_at(Screen::Wlan);
                }
                if self.state.check != Check::Asking {
                    self.state.check = Check::Asking;
                    self.refresh();
                    return vec![Effect::CheckUpdate];
                }
            }
            // The package goes to internal storage, which the decrypt page
            // opens, over a network the WLAN page joins.
            Action::Download => {
                let release = self.state.newest().expect("the download is offered for the newest system").clone();
                if !self.state.data.storage_open() {
                    self.open(Screen::Decrypt);
                    return self.arrive_at(Screen::Decrypt);
                }
                if !self.state.online() {
                    self.open(Screen::Wlan);
                    return self.arrive_at(Screen::Wlan);
                }
                if !self.state.downloading() {
                    self.state.download = Some(DownloadJob { progress: None, rate: 0, outcome: None, stopping: false });
                    self.refresh();
                    return vec![Effect::Download(release)];
                }
            }
            Action::StopDownload => {
                let job = self.state.download.as_mut().expect("only a download under way is stopped");
                if job.outcome.is_none() && !job.stopping {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopDownload];
                }
            }
            Action::InstallDownloaded => {
                let name = self.state.newest().expect("the package installed is the newest system's").name.clone();
                let Have::Whole(size) = self.state.have else { unreachable!("only a whole package is offered to install") };
                return self.install_file(Place::Storage, format!("{DOWNLOADS}/{name}"), size, now);
            }
            // Asking takes a network, which the WLAN page joins.
            Action::CheckSkyward => {
                if !self.state.online() {
                    self.open(Screen::Wlan);
                    return self.arrive_at(Screen::Wlan);
                }
                if self.state.skyward_check != SkywardCheck::Asking {
                    self.state.skyward_check = SkywardCheck::Asking;
                    self.refresh();
                    return vec![Effect::CheckSkyward];
                }
            }
            // The build goes into the recovery partitions an install may
            // write too, so one waits for the other.
            Action::UpdateSkyward => {
                let build = self.state.skyward_newest().and_then(|n| n.build.clone()).expect("the update is offered for the newest release's build");
                if !self.state.online() {
                    self.open(Screen::Wlan);
                    return self.arrive_at(Screen::Wlan);
                }
                if !matches!(self.state.install, Install::Idle | Install::Finished { .. }) {
                    self.say(l.pick(INSTALL_UNFINISHED), now);
                    return Vec::new();
                }
                if !self.state.updating_skyward() {
                    self.state.skyward_job = Some(SkywardJob { stage: None, rate: 0, outcome: None, stopping: false });
                    self.refresh();
                    return vec![Effect::UpdateSkyward(build)];
                }
            }
            // Only the download stops: writing, once begun, goes on to its
            // end.
            Action::StopSkywardUpdate => {
                let job = self.state.skyward_job.as_mut().expect("only an update under way is stopped");
                if job.outcome.is_none() && !job.stopping && !matches!(job.stage, Some(SkywardStage::Write { .. })) {
                    job.stopping = true;
                    self.refresh();
                    return vec![Effect::StopSkywardUpdate];
                }
            }
            // The OTG drive's files: listed on a page of their own, and an
            // entry in it opens as any other folder's does.
            Action::FilesOtg => {
                if self.state.place_open(Place::UsbDrive) {
                    self.state.otg = Folder::new("");
                    self.open(Screen::FilesOtg);
                    return vec![Effect::ReadOtg(String::new())];
                }
                self.say(l.pick(["未连接 U 盘", "未連接 USB 隨身碟", "No USB drive"]), now);
            }
            Action::OpenOtgEntry(i) => {
                let entry = self.state.otg.entry(i).clone();
                match entry.kind {
                    FileKind::Folder { .. } | FileKind::Link { folder: true, .. } => {
                        self.state.otg.leave(self.top().1);
                        return vec![Effect::ReadOtg(self.state.otg.inside(&entry.name))];
                    }
                    _ => {
                        self.state.file_on_drive = true;
                        self.state.file = Some(entry);
                        self.open(Screen::FileInfo);
                    }
                }
            }
            // Across to the other side, the file kept picked: the page it
            // goes to is opened so that the folder it lands in can be walked
            // to, and the bar at the foot puts it down there.
            Action::ToDrive(op) | Action::ToPhone(op) => {
                let to_drive = matches!(action, Action::ToDrive(_));
                if to_drive && !self.state.place_open(Place::UsbDrive) {
                    self.say(l.pick(["请先连接 U 盘", "請先連接 USB 隨身碟", "Connect a USB drive first"]), now);
                    return Vec::new();
                }
                let path = self.entry_path();
                // The entry's own folder is no place to put it down again, and
                // a folder cannot go inside itself.
                let here = match to_drive {
                    true => self.state.otg.path.trim_matches('/').to_string(),
                    false => self.state.files.path.clone(),
                };
                let nothing_to_do = match to_drive {
                    true => false,
                    false => op == FileOp::Move && std::path::Path::new(&path).parent() == Some(std::path::Path::new(&here)),
                };
                if nothing_to_do {
                    let name = std::path::Path::new(&path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                    self.say(&tr!(l, "\"{name}\"已在该文件夹中", "「{name}」已在這個資料夾中", "“{name}” is already in this folder"), now);
                    return Vec::new();
                }
                self.state.clip = Some(Clip { op, path });
                self.refresh();
                return match to_drive {
                    true => {
                        self.open(Screen::FilesOtg);
                        vec![Effect::ReadOtg(self.state.otg.path.clone())]
                    }
                    false => {
                        self.state.file_on_drive = false;
                        self.open(Screen::Files);
                        vec![Effect::ReadDir(self.state.files.path.clone())]
                    }
                };
            }
            // The apps page ticks one app, or every one, and works on those
            // ticked; taking them off the phone is asked for first.
            Action::AppsPick(i) => {
                if let Some(Ok(rows)) = &self.state.apps
                    && let Some(app) = rows.get(i)
                {
                    let label = app.label.clone();
                    if !self.state.chosen_apps.remove(&label) {
                        self.state.chosen_apps.insert(label);
                    }
                    self.refresh();
                }
            }
            Action::AppsAll => {
                let all: Vec<String> = match &self.state.apps {
                    Some(Ok(rows)) => rows.iter().map(|a| a.label.clone()).collect(),
                    _ => Vec::new(),
                };
                if !all.is_empty() && self.state.chosen_apps.len() == all.len() {
                    self.state.chosen_apps.clear();
                } else {
                    self.state.chosen_apps.extend(all);
                }
                self.refresh();
            }
            Action::AppsBackUp | Action::AppsRestore => {
                let task = match action {
                    Action::AppsRestore => AppTask::Restore,
                    _ => AppTask::BackUp,
                };
                let apps = ticked(&self.state);
                match apps.is_empty() {
                    true => self.say(l.pick(["先选择要处理的系统应用", "先選擇要處理的系統應用程式", "Pick the apps to work on first"]), now),
                    // Putting back a copy that is not kept is turned down
                    // here, in the user's words: nothing waits in the store to
                    // be put back.
                    false if task == AppTask::Restore && !any_kept(&self.state) => {
                        self.say(l.pick(["所选的系统应用没有备份", "所選的系統應用程式沒有備份", "The apps picked have no copy kept"]), now)
                    }
                    false => {
                        // The page turns to the work at once, and the runtime
                        // tells how far it has come.
                        let total = apps.len();
                        self.state.apps_work = Some(AppsWork { task, done: 0, total });
                        self.refresh();
                        return vec![Effect::AppWork { task, apps }];
                    }
                }
            }
            Action::AskAppsRemove => {
                let apps = ticked(&self.state);
                match apps.is_empty() {
                    true => self.say(l.pick(["先选择要删除的系统应用", "先選擇要刪除的系統應用程式", "Pick the apps to remove first"]), now),
                    false => {
                        self.state.asking_apps = true;
                        self.refresh();
                    }
                }
            }
            Action::Unavailable(text) => self.say(text, now),
        }
        Vec::new()
    }

    /// The text the keyboard types into on this page.
    fn typing(&mut self) -> &mut String {
        let screen = self.screen();
        match self.text_field() {
            Some(field) => field,
            None => panic!("{screen:?} has no text field"),
        }
    }

    /// The text field of the page shown, where it has one: the address on the
    /// download, browser and search pages, the lock's code, a new name, or a
    /// password.
    fn text_field(&mut self) -> Option<&mut String> {
        match self.screen() {
            Screen::Search => Some(&mut self.state.search),
            Screen::Decrypt => Some(&mut self.state.code),
            Screen::Rename => Some(&mut self.state.new_name),
            Screen::WlanPassword => Some(&mut self.state.password),
            Screen::Fetch => Some(&mut self.state.fetch_url),
            Screen::Browser => Some(&mut self.state.browse_url),
            Screen::WlanAdd => {
                let a = self.state.adding.as_mut()?;
                match a.field {
                    Field::Name => Some(&mut a.name),
                    Field::Password => Some(&mut self.state.password),
                }
            }
            _ => None,
        }
    }

    /// A key of the on-screen keyboard. A capital lasts one letter; return
    /// opens the first result of a search, and runs a terminal's command.
    fn type_key(&mut self, cap: Cap, now: u64) -> Vec<Effect> {
        let (l, layer) = (self.state.lang, self.state.layer);
        match cap {
            Cap::Char(ch) => {
                let shifted = self.state.layer == Layer::Shifted;
                self.typing().push(if shifted { ch.to_ascii_uppercase() } else { ch });
                self.edits += 1;
                if shifted {
                    self.state.layer = Layer::Letters;
                }
            }
            Cap::Space => {
                self.typing().push(' ');
                self.edits += 1;
            }
            Cap::Backspace => {
                self.typing().pop();
                self.edits += 1;
            }
            Cap::Shift => {
                self.state.layer = if self.state.layer == Layer::Shifted { Layer::Letters } else { Layer::Shifted };
            }
            Cap::Show(layer) => self.state.layer = layer,
            Cap::Enter => match self.screen() {
                Screen::Search => {
                    if let Some(first) = search::find_on(&self.state.search, l, |screen| self.state.offers(screen)).first() {
                        return self.act(Action::Open(first.screen), now);
                    }
                }
                // Return starts the download, or stops the one under way.
                Screen::Fetch if self.fetching() => return self.act(Action::FetchStop, now),
                Screen::Fetch => return self.act(Action::FetchStart, now),
                // Return opens the page, or stops the one being opened.
                Screen::Browser if self.state.browsing.loading() => return self.act(Action::BrowseStop, now),
                Screen::Browser => return self.act(Action::BrowseGo, now),
                Screen::Decrypt if self.state.code.is_empty() => self.say(l.pick(["请输入锁屏密码", "請輸入螢幕鎖定密碼", "Enter the screen lock"]), now),
                Screen::Decrypt => {
                    let code = std::mem::take(&mut self.state.code);
                    return self.unlock(code);
                }
                // A name the folder cannot take is turned down here, in
                // the user's words; an unchanged one leaves the page.
                Screen::Rename => {
                    let name = self.state.new_name.clone();
                    let current = &self.state.file.as_ref().expect("the file page shows an entry").name;
                    let taken = self.state.files.entries.as_ref().is_ok_and(|entries| entries.iter().any(|e| e.name == name));
                    match name.as_str() {
                        "" => self.say(l.pick(["请输入名称", "請輸入名稱", "Enter a name"]), now),
                        "." | ".." => self.say(l.pick(["名称不能为\".\"或\"..\"", "名稱不能是「.」或「..」", "A name can't be “.” or “..”"]), now),
                        _ if name.contains('/') => self.say(l.pick(["名称不能包含\"/\"", "名稱不能包含「/」", "A name can't hold “/”"]), now),
                        _ if name == *current => return self.back(now),
                        _ if taken => self.say_taken(now),
                        _ => return vec![Effect::Rename { path: self.entry_path(), name }],
                    }
                }
                // A password the network's protection cannot take is turned
                // down here; one it can goes, and the WLAN page shows how
                // joining goes.
                Screen::WlanPassword => {
                    let network = self.state.joining.clone().expect("the password page is for a network");
                    match refused(l, network.security, &self.state.password) {
                        Some(why) => self.say(why, now),
                        None => {
                            let password = Secret(std::mem::take(&mut self.state.password));
                            let mut fx = self.back(now);
                            fx.push(Effect::JoinWlan { ssid: network.ssid, name: network.name, security: network.security, password });
                            return fx;
                        }
                    }
                }
                // So is a name no network can have; the keyboard then types
                // into the field turned down.
                Screen::WlanAdd => {
                    let a = self.state.adding.as_mut().expect("the page adds a network");
                    let (field, why) = match a.name.len() {
                        0 => (Field::Name, Some(l.pick(["请输入网络名称", "請輸入網路名稱", "Enter the network's name"]).to_string())),
                        n if n > SSID_MAX => (
                            Field::Name,
                            Some(tr!(l, "网络名称不能超过 {SSID_MAX} 个字符", "網路名稱不能超過 {SSID_MAX} 個字元", "A network's name can't be over {SSID_MAX} characters")),
                        ),
                        _ if a.security.password() => (Field::Password, refused(l, a.security, &self.state.password).map(str::to_string)),
                        _ => (Field::Name, None),
                    };
                    match why {
                        Some(why) => {
                            if a.field != field {
                                a.field = field;
                                self.refresh();
                            }
                            self.say(&why, now)
                        }
                        None => {
                            let (name, security) = (a.name.clone(), a.security);
                            let password = match security.password() {
                                true => Secret(std::mem::take(&mut self.state.password)),
                                false => Secret::default(),
                            };
                            let mut fx = self.back(now);
                            fx.push(Effect::JoinWlan { ssid: name.clone().into_bytes(), name, security, password });
                            return fx;
                        }
                    }
                }
                other => unreachable!("{other:?} shows no keyboard"),
            },
        }
        // What is typed shows above the keyboard, which stays as it was
        // while its keys do.
        if matches!(cap, Cap::Char(_) | Cap::Space | Cap::Backspace) && self.state.layer == layer {
            let t = self.theme();
            self.refresh_rows(0, (t.m.h as f32 - widgets::keyboard_height(&t)).floor() as u32);
        } else {
            self.refresh();
        }
        Vec::new()
    }

    /// What a swipe-to-confirm control slid to its end does.
    pub(super) fn confirmed(&mut self, which: Confirm, now: u64) -> Vec<Effect> {
        let (l, mut fx) = (self.state.lang, Vec::new());
        // Every slider taken across is answered with a click of the motor,
        // whatever it was asking: the finger is told the question was taken
        // without looking at the page it was taken on.
        if self.state.vibrate {
            fx.push(Effect::Haptic);
        }
        match which {
            Confirm::Avb => {
                if let Some((verity, verification)) = self.state.avb_pending.take() {
                    fx.push(Effect::AvbSet(verity, verification));
                }
            }
            Confirm::Reboot => fx.push(Effect::Reboot(TARGETS[self.state.reboot])),
            // The slider taken across: the lock is kept from now on. The risk
            // is asked again the next time it is turned on.
            Confirm::KeepLock => {
                self.state.asking_remember = false;
                self.state.remember = true;
                self.refresh();
                fx.push(Effect::Remember(true));
            }
            Confirm::Wipe => {
                let chosen: Vec<Wipe> = WIPES.iter().zip(self.state.wipe).filter(|(_, on)| *on).map(|(&w, _)| w).collect();
                match chosen.is_empty() {
                    true => self.say(l.pick(["请选择要清除的内容", "請選擇要清除的內容", "Choose what to wipe"]), now),
                    false => fx.push(Effect::Wipe(chosen)),
                }
            }
            Confirm::Install | Confirm::InstallWipe => {
                if let Install::Confirming { version, .. } = &self.state.install {
                    self.state.install = Install::Updating { version: version.clone(), stage: None, overwriting: false };
                    fx.push(Effect::Install);
                    self.refresh();
                }
            }
            Confirm::RunScript => {
                if self.state.install == Install::ConfirmingScript {
                    self.state.install = Install::Scripting { stage: None, running: false };
                    fx.push(Effect::Install);
                    self.refresh();
                }
            }
            Confirm::Backup => {
                let s = &self.state;
                let refused = if !s.backup.contains(&true) {
                    Some(["请选择要备份的内容", "請選擇要備份的內容", "Choose what to back up"])
                } else if s.backup[2] && s.data != Data::Open {
                    Some(["备份 data 分区需先完成「解密」", "備份 data 分割區需先完成「解密」", "Decrypt before backing up the data partition"])
                } else if s.place_open(s.backup_to) {
                    None
                } else {
                    Some(match (s.backup_to, &s.usb_drive) {
                        (Place::Storage, _) if s.data == Data::Erased => DATA_ERASED,
                        (Place::Storage, _) => DECRYPT_FIRST,
                        (Place::UsbDrive, Some(UsbDrive::Unreadable(_))) => USB_DRIVE_UNREADABLE,
                        (Place::UsbDrive, _) => USB_DRIVE_WANTED,
                    })
                };
                let backup = Effect::Backup { parts: s.backup, place: s.backup_to, compress: s.compress, verify: s.verify };
                match refused {
                    Some(why) => self.say(l.pick(why), now),
                    None => fx.push(backup),
                }
            }
            Confirm::Restore => {
                let (entry, parts) = self.state.restore.as_ref().expect("the restore page shows a backup");
                let restore = Effect::Restore { place: entry.place, name: entry.name.clone(), parts: *parts };
                if !parts.contains(&true) {
                    self.say(l.pick(["请选择要恢复的内容", "請選擇要還原的內容", "Choose what to restore"]), now);
                } else if parts[2] && self.state.data != Data::Open {
                    self.say(l.pick(["恢复 data 分区需先完成「解密」", "還原 data 分割區需先完成「解密」", "Decrypt before restoring the data partition"]), now);
                } else {
                    fx.push(restore);
                }
            }
            Confirm::DeleteBackup => {
                let (entry, _) = self.state.restore.as_ref().expect("the restore page shows a backup");
                fx.push(Effect::DeleteBackup { place: entry.place, name: entry.name.clone() });
            }
            Confirm::Slot => match (&self.state.boot_slots, self.state.next_slot) {
                (Some(Ok(slots)), Some(pick)) if pick != slots.active => fx.push(Effect::SetSlot(pick)),
                (Some(Ok(slots)), _) => {
                    let slot = slots.active.letter();
                    let said = tr!(l, "下次已从槽位 {slot} 启动", "下次已從槽位 {slot} 啟動", "Slot {slot} already starts next");
                    self.say(&said, now);
                }
                _ => self.say(l.pick(["无法读取启动槽位", "無法讀取啟動槽位", "Couldn't read the boot slots"]), now),
            },
            Confirm::FactoryReset => fx.push(Effect::FactoryReset),
            Confirm::Merge => {
                if self.state.update == Standing::Waiting {
                    self.state.update = Standing::Merging(0.0);
                    self.refresh();
                    fx.push(Effect::Merge);
                }
            }
            Confirm::DeleteEntry => match self.state.asking_many.take() {
                Some(paths) => {
                    self.state.choosing = false;
                    self.state.chosen.clear();
                    self.refresh();
                    fx.push(Effect::DeleteMany(paths));
                }
                None => fx.push(Effect::Delete(self.entry_path())),
            },
            // The partition has to be open; whether its users have unlocked
            // their own files makes no difference.
            Confirm::Relabel => match self.state.data {
                Data::Open | Data::Locked { .. } => fx.push(Effect::Relabel),
                Data::Opening => self.say(l.pick(DATA_OPENING), now),
                Data::Erased => self.say(l.pick(DATA_ERASED), now),
                Data::Closed | Data::Failed(_) => {
                    self.say(l.pick(["请先在「解密」中打开 data 分区", "請先在「解密」中開啟 data 分割區", "Open the data partition in Decrypt first"]), now)
                }
            },
            // The data partition is reached through its own key whether or
            // not it is open, but not while it is being opened or erased.
            Confirm::Filesystem => {
                let row = self.fs_row().expect("the page of a partition shows one that was read");
                let data = row.name == DATA_PARTITION;
                match (&self.state.fs_action, &self.state.data) {
                    (_, Data::Opening) if data => self.say(l.pick(DATA_OPENING), now),
                    (FsAction::Repair, Data::Erased) if data => self.say(l.pick(DATA_ERASED), now),
                    (action, _) => fx.push(Effect::Filesystem { name: row.name.clone(), action: action.clone() }),
                }
            }
            Confirm::Flash => {
                if let Install::Choosing(choice) = &self.state.install {
                    let partition = choice.partitions[choice.partition];
                    fx.push(Effect::Flash { partition, pick: choice.pick });
                    self.state.install = Install::Flashing { partition, stage: None };
                    self.refresh();
                }
            }
            Confirm::SwitchSystem => {
                if let (Some(Ok(systems)), Some(pick)) = (&self.state.systems, self.state.system_pick)
                    && pick != starting(systems)
                {
                    fx.push(Effect::SwitchSystem(pick));
                    self.state.systems_work = Some(SystemsWork::Switching(pick));
                    self.state.system_pick = None;
                    self.refresh();
                }
            }
            Confirm::RemoveSystem => {
                fx.push(Effect::RemoveSystem);
                self.state.systems_work = Some(SystemsWork::Removing);
                self.state.asking_delete = false;
                self.state.system_pick = None;
                self.refresh();
            }
            Confirm::InstallSystem => {
                if let Install::SystemChoosing(choice) = &self.state.install {
                    fx.push(Effect::InstallSystem { size: choice.sizes[choice.size] });
                    self.state.install = Install::Systeming { name: choice.name.clone(), stage: None };
                    self.refresh();
                }
            }
        Confirm::RemoveApps => {
                let apps = ticked(&self.state);
                if !apps.is_empty() {
                    let total = apps.len();
                    fx.push(Effect::AppWork { task: AppTask::Remove, apps });
                    self.state.apps_work = Some(AppsWork { task: AppTask::Remove, done: 0, total });
                    self.state.asking_apps = false;
                    self.refresh();
                }
            }
        }
        fx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;

    #[test]
    fn search_finds_by_pinyin_typed_on_its_own_keyboard() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Search, &f, &i);
        type_text(&mut app, &f, &i, "bf");
        assert_eq!(app.state.search, "bf");
        let opens: Vec<Action> = app.frame(&f, &i).hits.iter().map(|h| h.action).filter(|a| matches!(a, Action::Open(_))).collect();
        assert_eq!(opens, [Action::Open(Screen::Backup)]);
        tap(&mut app, &f, &i, Action::Type(Cap::Backspace));
        assert_eq!(app.state.search, "b");
        tap(&mut app, &f, &i, Action::ClearSearch);
        assert_eq!(app.state.search, "");
        type_text(&mut app, &f, &i, "xpsj");
        tap(&mut app, &f, &i, Action::Type(Cap::Enter));
        assert_eq!(app.screen(), Screen::ScreenOff, "return opens the first result");
        app.key(Key::Back, 0);
        assert_eq!(app.screen(), Screen::Search);
    }

    #[test]
    fn shift_makes_one_capital_and_the_layers_switch() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Search, &f, &i);
        tap(&mut app, &f, &i, Action::Type(Cap::Shift));
        type_text(&mut app, &f, &i, "ab");
        assert_eq!(app.state.search, "Ab");
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Digits)));
        type_text(&mut app, &f, &i, "1");
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Symbols)));
        type_text(&mut app, &f, &i, "|");
        tap(&mut app, &f, &i, Action::Type(Cap::Space));
        assert_eq!(app.state.search, "Ab1| ");
        tap(&mut app, &f, &i, Action::Type(Cap::Show(Layer::Letters)));
        assert_eq!(app.state.layer, Layer::Letters);
    }

    #[test]
    fn a_key_shows_held_down_while_touched() {
        let (f, i) = (fonts(), info());
        let mut app = opened(Screen::Search, &f, &i);
        let (x, y) = spot(&mut app, &f, &i, Action::Type(Cap::Char('q')));
        app.touch(Touch::Down(x, y), &f, 0);
        assert_eq!(app.state.key_held(), Some(Cap::Char('q')));
        app.touch(Touch::Up, &f, 0);
        assert_eq!(app.state.held, None);
        assert_eq!(app.state.search, "q");
    }
}
