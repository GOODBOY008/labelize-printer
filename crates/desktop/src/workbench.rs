use crate::{
    i18n::Language,
    studio::Studio,
    studio_worker::Preview,
    ui::{ACCENT, BORDER, MUTED},
};
use gpui::{prelude::*, *};
use gpui_component::{
    button::{Button, ButtonVariants},
    Disableable, IconName, Sizable,
};
use labelize_printer::{
    preview_images::PreviewCache,
    preview_queue::PreviewQueue,
    session::{self, Draft, SessionStore, Workspace},
};
use labelize_printer_core::{
    design::DesignDocument,
    document::{self, DocumentKind},
    AppPaths, OutputFormat, Settings,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

enum Notice {
    Restored,
    Message(String),
}

pub struct Workbench {
    pub language: Language,
    paths: AppPaths,
    defaults: Settings,
    tabs: Vec<Entity<Studio>>,
    active: Option<String>,
    pub recent: Vec<PathBuf>,
    pub setup_dismissed: bool,
    store: Arc<SessionStore>,
    queue: Arc<PreviewQueue<Preview>>,
    images: Arc<Mutex<PreviewCache>>,
    sequence: u64,
    versions: HashMap<String, u64>,
    save_due: Option<Instant>,
    saving: bool,
    restoring: bool,
    loading: HashSet<PathBuf>,
    pending_open: Vec<PathBuf>,
    session_error: Option<String>,
    recovery_warning: bool,
    notice: Option<Notice>,
    last_saved: bool,
    menu: bool,
    documents_menu: bool,
    visible: bool,
    tab_scroll: ScrollHandle,
    focus: FocusHandle,
}
impl Workbench {
    pub fn new(
        paths: AppPaths,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = Arc::new(SessionStore::new(paths.root.clone()));
        let read = store.clone();
        let load = cx.background_executor().spawn(async move { read.load() });
        cx.spawn_in(window, async move |this, cx| {
            let (workspace, warnings) = load.await;
            let _ = this.update_in(cx, |s, window, cx| {
                s.restoring = false;
                s.recent = workspace.recent;
                s.setup_dismissed = workspace.setup_dismissed;
                let active = workspace.active;
                for draft in workspace.tabs { s.insert(draft, window, cx); }
                if let Some(id) = active { s.activate(&id, window, cx); }
                if !warnings.is_empty() {
                    s.recovery_warning = true;
                    s.session_error = Some(format!("{}\n{}", s.t(
                        "部分工作区数据无法恢复；原始快照保留在本机。",
                        "Some workspace data could not be restored; original snapshots remain on disk."
                    ), warnings.join("\n")));
                } else if !s.tabs.is_empty() {
                    s.notice = Some(Notice::Restored);
                    s.last_saved = true;
                }
                for path in std::mem::take(&mut s.pending_open) { s.open_path(path, window, cx); }
                cx.notify();
            });
        }).detach();
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            if this.update_in(cx, |s, w, cx| s.tick(w, cx)).is_err() {
                break;
            }
        })
        .detach();
        Self {
            language,
            paths,
            defaults: Settings::default(),
            tabs: Vec::new(),
            active: None,
            recent: Vec::new(),
            setup_dismissed: false,
            store,
            queue: Arc::new(PreviewQueue::new()),
            images: Arc::new(Mutex::new(PreviewCache::default())),
            sequence: 0,
            versions: HashMap::new(),
            save_due: None,
            saving: false,
            restoring: true,
            loading: HashSet::new(),
            pending_open: Vec::new(),
            session_error: None,
            recovery_warning: false,
            notice: None,
            last_saved: false,
            menu: false,
            documents_menu: false,
            visible: true,
            tab_scroll: ScrollHandle::default(),
            focus: cx.focus_handle(),
        }
    }
    pub fn t(&self, zh: &'static str, en: &'static str) -> &'static str {
        match self.language {
            Language::Chinese => zh,
            Language::English => en,
        }
    }
    pub fn configure(&mut self, settings: &Settings, _: &mut Window, _: &mut Context<Self>) {
        self.defaults = settings.clone();
    }
    pub fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        self.language = language;
        for tab in &self.tabs {
            tab.update(cx, |s, cx| {
                s.language = language;
                cx.notify();
            });
        }
        cx.notify();
    }
    pub fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        for tab in &self.tabs {
            let active = visible && Some(&tab.read(cx).id) == self.active.as_ref();
            tab.update(cx, |s, cx| s.set_active(active, window, cx));
        }
    }
    pub fn dismiss_setup(&mut self, cx: &mut Context<Self>) {
        self.setup_dismissed = true;
        self.changed(cx);
    }
    fn active_tab(&self, cx: &App) -> Option<Entity<Studio>> {
        self.tabs
            .iter()
            .find(|t| Some(&t.read(cx).id) == self.active.as_ref())
            .cloned()
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.sequence += 1;
        self.save_due = Some(Instant::now() + Duration::from_secs(1));
        self.last_saved = false;
        cx.notify();
    }
    fn snapshot(&self, cx: &App) -> Workspace {
        Workspace {
            tabs: self.tabs.iter().map(|t| t.read(cx).snapshot(cx)).collect(),
            active: self.active.clone(),
            recent: self.recent.clone(),
            setup_dismissed: self.setup_dismissed,
        }
    }
    fn insert(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        let id = draft.id.clone();
        let queue = self.queue.clone();
        let paths = self.paths.clone();
        let language = self.language;
        let images = self.images.clone();
        let tab = cx.new(|cx| Studio::new(paths, language, draft, queue, images, window, cx));
        self.versions.insert(id.clone(), 0);
        cx.observe_in(&tab, window, |s, tab, w, cx| {
            let doc = tab.read(cx);
            let id = doc.id.clone();
            let version = doc.persist_version;
            let closed = doc.close_ready;
            let file = doc.file.clone();
            if closed {
                s.remove(&id, w, cx);
                return;
            }
            if s.versions.get(&id) != Some(&version) {
                s.versions.insert(id, version);
                if let Some(path) = file {
                    session::remember_recent(&mut s.recent, path);
                }
                s.changed(cx);
            }
            // Invalidate only for the visible document. Root schedules one
            // frame after notifications; refreshing here would bypass it and
            // can leave GPUI 0.2.2 painting a scene with evicted textures.
            if s.visible && s.active.as_deref() == Some(&tab.read(cx).id) {
                cx.notify();
            }
        })
        .detach();
        self.tabs.push(tab);
        self.activate(&id, window, cx);
    }
    fn activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.iter().any(|t| t.read(cx).id == id) {
            return;
        }
        let changed = self.active.as_deref() != Some(id);
        self.active = Some(id.into());
        self.queue.activate(self.active.clone());
        if changed {
            for tab in &self.tabs {
                let active = self.visible && tab.read(cx).id == id;
                tab.update(cx, |s, cx| s.set_active(active, window, cx));
            }
        }
        self.documents_menu = false;
        self.focus.focus(window);
        if let Some(tab) = self.active_tab(cx) {
            tab.update(cx, |s, cx| s.focus_editor(window, cx));
        }
        if changed {
            self.changed(cx);
        }
        cx.notify();
    }
    fn remove(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let index = self
            .tabs
            .iter()
            .position(|t| t.read(cx).id == id)
            .unwrap_or(0);
        self.queue.cancel(id);
        self.tabs.retain(|t| t.read(cx).id != id);
        self.versions.remove(id);
        if self.active.as_deref() == Some(id) {
            self.active = None;
            if let Some(tab) = self.tabs.get(index.min(self.tabs.len().saturating_sub(1))) {
                let id = tab.read(cx).id.clone();
                self.activate(&id, window, cx);
            }
        }
        self.changed(cx);
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.restoring {
            return;
        }
        if let Some(tab) = self.active_tab(cx) {
            tab.update(cx, |s, cx| s.tick(window, cx));
        }
        if !self.saving
            && self.session_error.is_none()
            && self.save_due.is_some_and(|due| Instant::now() >= due)
        {
            self.save_due = None;
            self.saving = true;
            let sequence = self.sequence;
            let snapshot = self.snapshot(cx);
            let store = self.store.clone();
            let save = cx
                .background_executor()
                .spawn(async move { store.save(sequence, snapshot) });
            cx.spawn_in(window, async move |this, cx| {
                let result = save.await;
                let _ = this.update_in(cx, |s, _, cx| {
                    s.saving = false;
                    match result {
                        Ok(()) => {
                            s.last_saved = s.sequence == sequence;
                        }
                        Err(e) => {
                            s.session_error = Some(format!(
                                "{} {e:#}",
                                s.t("本机草稿暂存失败。", "Could not save local drafts.")
                            ));
                            s.save_due = Some(Instant::now());
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }
    pub fn resume_update_previews(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queue.resume();
        // Re-arm any stale active document cancelled by update preparation.
        self.set_visible(self.visible, window, cx);
    }
    pub fn update_previews(&self) -> Arc<PreviewQueue<Preview>> {
        self.queue.clone()
    }
    pub fn request_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let pending: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let doc = tab.read(cx);
                if doc.is_saving() || doc.is_submitting() {
                    Some(doc.title())
                } else {
                    None
                }
            })
            .collect();
        if self.restoring || !self.loading.is_empty() || !pending.is_empty() {
            let detail = if self.restoring {
                self.t("正在恢复工作区", "Restoring workspace").to_string()
            } else if !self.loading.is_empty() {
                self.t("正在打开文件", "Opening files").to_string()
            } else {
                pending.join(", ")
            };
            self.notice = Some(Notice::Message(format!(
                "{} {detail}",
                self.t(
                    "请等待文件操作或任务提交完成后再关闭：",
                    "Wait for the file operation or job submission before closing:"
                )
            )));
            cx.notify();
            return false;
        }
        // A failed recovery needs acknowledgement before overwriting any snapshot.
        if self.session_error.is_some() {
            cx.notify();
            return false;
        }
        self.sequence += 1;
        match self.store.save(self.sequence, self.snapshot(cx)) {
            Ok(()) => true,
            Err(e) => {
                self.session_error=Some(format!("{} {e:#}",self.t("无法暂存草稿，窗口暂未关闭。请检查磁盘并重试。", "Drafts could not be saved. The window remains open; check disk access and retry.")));
                cx.notify();
                false
            }
        }
    }
    pub fn new_design(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.restoring {
            return;
        }
        self.insert(Draft::new(self.defaults.clone()), window, cx);
        self.changed(cx);
    }
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(self.t("打开标签文件", "Open label files").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = choice.await;
            let _ = this.update_in(cx, |s, w, cx| {
                match paths {
                    Ok(Ok(Some(paths))) => {
                        for path in paths {
                            s.open_path(path, w, cx);
                        }
                    }
                    Ok(Err(e)) => s.notice = Some(Notice::Message(e.to_string())),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.restoring {
            self.pending_open.push(path);
            return;
        }
        let path = session::canonical_path(&path);
        if let Some(tab) = self
            .tabs
            .iter()
            .find(|t| t.read(cx).file.as_ref() == Some(&path))
        {
            let id = tab.read(cx).id.clone();
            self.activate(&id, window, cx);
            return;
        }
        if !self.loading.insert(path.clone()) {
            return;
        }
        let target = path.clone();
        let settings = self.defaults.clone();
        let read = cx.background_executor().spawn(async move {
            let bytes = document::read_input(&target)?;
            let mut draft = Draft::new(settings);
            draft.kind = DocumentKind::detect(&target, &bytes);
            draft.design = if draft.kind == DocumentKind::Design {
                let d: DesignDocument = serde_json::from_slice(&bytes)?;
                d.validate(draft.options.dpmm)?;
                Some(d)
            } else {
                None
            };
            draft.fingerprint = Some(session::digest(&bytes));
            draft.file = Some(target);
            draft.source = if draft.design.is_none() {
                bytes
            } else {
                Vec::new()
            };
            draft.dirty = false;
            draft.selected = draft
                .design
                .as_ref()
                .and_then(|d| (!d.objects.is_empty()).then_some(0));
            Ok::<_, anyhow::Error>(draft)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            let _ = this.update_in(cx, |s, w, cx| {
                s.loading.remove(&path);
                match result {
                    Ok(draft) => {
                        session::remember_recent(&mut s.recent, path);
                        s.insert(draft, w, cx);
                        s.changed(cx);
                    }
                    Err(e) => {
                        s.notice = Some(Notice::Message(format!("{}: {e:#}", path.display())))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.active_tab(cx) {
            t.update(cx, |s, cx| s.save(window, cx));
        }
    }
    pub fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.active_tab(cx) {
            t.update(cx, |s, cx| s.save_as(true, window, cx));
        }
    }
    pub fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.active_tab(cx) {
            t.update(cx, |s, cx| s.export(window, cx));
        }
    }
    pub fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.active_tab(cx) {
            t.update(cx, |s, cx| s.request_tab_close(window, cx));
        }
    }
    pub fn cycle_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        if n == 0 {
            return;
        }
        let current = self
            .tabs
            .iter()
            .position(|t| Some(&t.read(cx).id) == self.active.as_ref())
            .unwrap_or(0);
        let i = (current + if back { n - 1 } else { 1 }) % n;
        let id = self.tabs[i].read(cx).id.clone();
        self.activate(&id, window, cx);
    }
    pub fn recent_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_1();
        if self.recent.is_empty() {
            list = list.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(self.t("还没有最近文件", "No recent files yet")),
            );
        }
        for (i, path) in self.recent.iter().enumerate() {
            let path = path.clone();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            list = list.child(
                div()
                    .id(("recent-file", i))
                    .p_3()
                    .cursor_pointer()
                    .rounded_md()
                    .hover(|d| d.bg(rgb(0xeef3ff)))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(gpui_component::Icon::new(IconName::File).text_color(rgb(ACCENT)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_sm().child(name))
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(path.to_string_lossy().into_owned()),
                            ),
                    )
                    .on_click(cx.listener(move |s, _, w, cx| s.open_path(path.clone(), w, cx))),
            );
        }
        list.into_any_element()
    }
}
impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = window.viewport_size().width < px(700.);
        let short = window.viewport_size().height < px(500.);
        let active = self.active_tab(cx);
        let mut tabs = div()
            .id("document-tabs-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll)
            .child({
                let mut row = div().flex().h_full();
                for (i, tab) in self.tabs.iter().enumerate() {
                    let doc = tab.read(cx);
                    let id = doc.id.clone();
                    let close = tab.clone();
                    row = row.child(
                        div()
                            .id(("document-tab", i))
                            .h_full()
                            .min_w(px(140.))
                            .max_w(px(228.))
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .border_r_1()
                            .border_b_2()
                            .border_color(rgb(if Some(&id) == self.active.as_ref() {
                                ACCENT
                            } else {
                                BORDER
                            }))
                            .bg(rgb(if Some(&id) == self.active.as_ref() {
                                0xffffff
                            } else {
                                0xf6f8fc
                            }))
                            .child(
                                gpui_component::Icon::new(IconName::File)
                                    .size(px(14.))
                                    .text_color(rgb(ACCENT)),
                            )
                            .child(div().text_sm().truncate().flex_1().child(doc.title()))
                            .when(doc.dirty, |d| {
                                d.child(div().size(px(6.)).rounded_full().bg(rgb(ACCENT)))
                            })
                            .child(
                                Button::new(("close-tab", i))
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .tooltip(self.t("关闭标签页", "Close tab"))
                                    .on_click(move |_, w, cx| {
                                        cx.stop_propagation();
                                        close.update(cx, |s, cx| s.request_tab_close(w, cx));
                                    }),
                            )
                            .on_click(cx.listener(move |s, _, w, cx| s.activate(&id, w, cx))),
                    );
                }
                row
            });
        if self.tabs.is_empty() {
            tabs = tabs.child(
                div()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(self.t("工作台", "Workspace")),
            );
        }
        let tabbar = div()
            .h(px(40.))
            .flex()
            .items_center()
            .flex_shrink_0()
            .bg(rgb(0xf6f8fc))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(tabs)
            .child(
                Button::new("new-tab")
                    .small()
                    .ghost()
                    .icon(IconName::Plus)
                    .tooltip(self.t("新建设计", "New design"))
                    .disabled(self.restoring)
                    .on_click(cx.listener(|s, _, w, cx| s.new_design(w, cx))),
            )
            .child(
                Button::new("document-list")
                    .small()
                    .ghost()
                    .icon(IconName::ChevronDown)
                    .tooltip(self.t("所有标签页", "All tabs"))
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.documents_menu = !s.documents_menu;
                        s.menu = false;
                        cx.notify();
                    })),
            );
        let mut actions = div().flex().items_center().gap_2();
        if !narrow {
            actions = actions
                .child(
                    Button::new("open-label")
                        .small()
                        .ghost()
                        .icon(IconName::FolderOpen)
                        .label(self.t("打开文件", "Open file"))
                        .on_click(cx.listener(|s, _, w, cx| s.open(w, cx))),
                )
                .child(
                    Button::new("new-design")
                        .small()
                        .ghost()
                        .icon(IconName::Plus)
                        .label(self.t("新建设计", "New design"))
                        .disabled(self.restoring)
                        .on_click(cx.listener(|s, _, w, cx| s.new_design(w, cx))),
                );
        }
        actions = actions
            .child(
                Button::new("save-label")
                    .small()
                    .label(self.t("保存", "Save"))
                    .disabled(active.as_ref().is_none_or(|t| t.read(cx).is_saving()))
                    .on_click(cx.listener(|s, _, w, cx| s.save(w, cx))),
            )
            .child(
                Button::new("document-menu")
                    .small()
                    .ghost()
                    .icon(IconName::Ellipsis)
                    .tooltip(self.t("更多操作", "More actions"))
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.menu = !s.menu;
                        s.documents_menu = false;
                        cx.notify();
                    })),
            );
        let format = active
            .as_ref()
            .map_or(OutputFormat::Pdf, |t| t.read(cx).format());
        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .h(px(if short { 40. } else { 52. }))
            .px_3()
            .flex_shrink_0()
            .bg(rgb(0xffffff))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(actions)
            .child(
                Button::new("export-document")
                    .small()
                    .primary()
                    .label(match format {
                        OutputFormat::Pdf => self.t("导出 PDF", "Export PDF"),
                        OutputFormat::Png => self.t("导出 PNG", "Export PNG"),
                    })
                    .disabled(active.as_ref().is_none_or(|t| t.read(cx).is_submitting()))
                    .on_click(cx.listener(|s, _, w, cx| s.export(w, cx))),
            );
        let mut root = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && (s.menu || s.documents_menu) {
                    s.menu = false;
                    s.documents_menu = false;
                    s.focus.focus(w);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .child(tabbar)
            .child(toolbar);
        if let Some(error) = &self.session_error {
            root = root.child(
                div()
                    .p_3()
                    .bg(rgb(0xffeeee))
                    .text_xs()
                    .child(error.clone())
                    .child(
                        Button::new("retry-session")
                            .small()
                            .label(if self.recovery_warning {
                                self.t("保留现有快照并重试", "Keep snapshots and retry")
                            } else {
                                self.t("重试暂存", "Retry saving drafts")
                            })
                            .on_click(cx.listener(|s, _, _, cx| {
                                // Preserve a complete forensic copy before acknowledging recovery errors.
                                let root = s.paths.root.join("workspace");
                                let archive = s
                                    .paths
                                    .root
                                    .join(format!("workspace-recovery-{}", uuid::Uuid::new_v4()));
                                let result = if s.recovery_warning && root.exists() {
                                    std::fs::rename(&root, archive)
                                } else {
                                    Ok(())
                                };
                                match result {
                                    Ok(()) => {
                                        s.session_error = None;
                                        s.recovery_warning = false;
                                        s.changed(cx);
                                    }
                                    Err(e) => s.session_error = Some(e.to_string()),
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(notice) = &self.notice {
            let message = match notice {
                Notice::Restored => self.t(
                    "已恢复上次工作区；未保存内容仍在本机草稿中。",
                    "Workspace restored. Unsaved changes are kept in local drafts.",
                ),
                Notice::Message(message) => message,
            };
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .bg(rgb(0xeef3ff))
                    .child(div().flex_1().child(message.to_owned()))
                    .child(
                        Button::new("dismiss-workspace-notice")
                            .small()
                            .ghost()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.notice = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(active) = active {
            root = root.child(div().flex_1().min_h_0().min_w_0().child(active));
        } else {
            root=root.child(div().id("empty-workspace-scroll").flex_1().min_h_0().overflow_y_scroll().child(div().w_full().max_w(px(700.)).mx_auto().px_6().py_8().flex().flex_col().gap_5()
                .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child(self.t("从一个标签开始", "Start with a label")))
                .child(div().text_sm().text_color(rgb(MUTED)).child(self.t("打开打印语言文件，或创建可视化设计。预览后导出 PDF / PNG。", "Open a printer-language file or create a visual design. Preview and export to PDF / PNG.")))
                .child(div().flex().flex_wrap().gap_3().child(Button::new("empty-open").primary().icon(IconName::FolderOpen).label(self.t("打开文件…", "Open file…")).on_click(cx.listener(|s,_,w,cx|s.open(w,cx))))
                    .child(Button::new("empty-new").icon(IconName::Plus).label(self.t("新建设计", "New design")).disabled(self.restoring).on_click(cx.listener(|s,_,w,cx|s.new_design(w,cx)))))
                .child(div().text_xs().text_color(rgb(MUTED)).child("ZPL · EPL · TSPL · CPCL · DPL · .labelize.json"))
                .child(div().pt_3().text_sm().font_weight(FontWeight::SEMIBOLD).child(self.t("最近文件", "Recent files"))).child(self.recent_view(cx))));
        }
        root = root.child(
            div()
                .h(px(24.))
                .flex_shrink_0()
                .px_3()
                .flex()
                .items_center()
                .bg(rgb(0xf9fbff))
                .border_t_1()
                .border_color(rgb(BORDER))
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child(if self.restoring {
                    self.t("正在恢复工作区…", "Restoring workspace…")
                } else if !self.loading.is_empty() {
                    self.t("正在打开文件…", "Opening files…")
                } else if self.last_saved {
                    self.t(
                        "工作区已暂存至本机 · 草稿不等于保存源文件",
                        "Workspace saved locally · drafts do not overwrite source files",
                    )
                } else {
                    self.t(
                        "修改将在本机自动暂存",
                        "Changes are saved to local drafts automatically",
                    )
                }),
        );
        if self.menu || self.documents_menu {
            root = root.child(div().absolute().inset_0().occlude().on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, _, w, cx| {
                    s.focus.focus(w);
                    s.menu = false;
                    s.documents_menu = false;
                    cx.notify();
                }),
            ));
            let mut menu = div()
                .id("workspace-menu")
                .absolute()
                .top(px(if self.documents_menu {
                    40.
                } else if short {
                    80.
                } else {
                    92.
                }))
                .right(px(12.))
                .w(px(260.))
                .max_h((window.viewport_size().height - px(150.)).min(px(320.)))
                .overflow_y_scroll()
                .occlude()
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_lg()
                .bg(rgb(0xffffff))
                .border_1()
                .border_color(rgb(BORDER))
                .shadow_lg();
            if self.documents_menu {
                for (i, tab) in self.tabs.iter().enumerate() {
                    let doc = tab.read(cx);
                    let id = doc.id.clone();
                    menu = menu.child(
                        Button::new(("switch-document", i))
                            .ghost()
                            .w_full()
                            .justify_start()
                            .label(format!(
                                "{}{}",
                                doc.title(),
                                if doc.dirty { " •" } else { "" }
                            ))
                            .on_click(cx.listener(move |s, _, w, cx| s.activate(&id, w, cx))),
                    );
                }
            } else {
                menu = menu
                    .child(
                        Button::new("menu-open")
                            .ghost()
                            .justify_start()
                            .label(self.t("打开文件…", "Open file…"))
                            .on_click(cx.listener(|s, _, w, cx| {
                                s.menu = false;
                                s.focus.focus(w);
                                s.open(w, cx);
                            })),
                    )
                    .child(
                        Button::new("menu-new")
                            .ghost()
                            .justify_start()
                            .label(self.t("新建设计", "New design"))
                            .on_click(cx.listener(|s, _, w, cx| {
                                s.menu = false;
                                s.focus.focus(w);
                                s.new_design(w, cx);
                            })),
                    )
                    .child(
                        Button::new("menu-save-as")
                            .ghost()
                            .justify_start()
                            .label(self.t("另存为…", "Save as…"))
                            .on_click(cx.listener(|s, _, w, cx| {
                                s.menu = false;
                                s.focus.focus(w);
                                s.save_as(w, cx);
                            })),
                    )
                    .child(
                        Button::new("menu-close")
                            .ghost()
                            .justify_start()
                            .label(self.t("关闭标签页", "Close tab"))
                            .on_click(cx.listener(|s, _, w, cx| {
                                s.menu = false;
                                s.focus.focus(w);
                                s.close_tab(w, cx);
                            })),
                    );
            }
            root = root.child(menu);
        }
        root.on_drop(cx.listener(|s, paths: &ExternalPaths, w, cx| {
            for path in paths.paths() {
                s.open_path(path.clone(), w, cx);
            }
        }))
    }
}
