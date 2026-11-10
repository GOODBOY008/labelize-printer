#[path = "ui_updates.rs"]
mod update_ui;
use crate::branding;
use crate::i18n::{Language, Preferences, Text};
use crate::worker::{self, Activity, Event, Work};
use gpui::{prelude::*, *};
#[cfg(windows)]
use gpui_component::select::{Select, SelectEvent, SelectState};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    scroll::ScrollableElement,
    Disableable, Icon, IconName, Selectable, Sizable, TitleBar,
};
use labelize_printer_core::{
    document::DocumentKind, AppPaths, JobState, OutputFormat, PrintJobInput, RenderMode, Settings,
    Snapshot,
};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
};

actions!(
    labelize_printer,
    [
        ShowStudio,
        OpenLabel,
        NewLabel,
        SaveLabel,
        SaveLabelAs,
        ExportLabel,
        CloseLabel,
        NextLabel,
        PreviousLabel,
        UndoLabel,
        RedoLabel,
        ShowRecent,
        ShowSetup,
        ShowSettings,
        ShowJobs,
        ScrollPageUp,
        ScrollPageDown
    ]
);

pub(crate) const ACCENT: u32 = 0x315efb;
pub(crate) const TEXT: u32 = 0x17243b;
pub(crate) const MUTED: u32 = 0x57657a;
pub(crate) const BORDER: u32 = 0xe0e6ef;
pub(crate) const SURFACE: u32 = 0xf5f7fb;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Studio,
    Recent,
    Setup,
    Settings,
    Updates,
    Jobs,
}

impl Page {
    fn title(self, language: Language) -> &'static str {
        match self {
            Self::Studio => language.text(Text::Studio),
            Self::Recent => match language {
                Language::Chinese => "最近文件",
                Language::English => "Recent files",
            },
            Self::Setup => language.text(Text::Setup),
            Self::Settings => language.text(Text::Settings),
            Self::Updates => match language {
                Language::Chinese => "软件更新",
                Language::English => "Software update",
            },
            Self::Jobs => language.text(Text::Jobs),
        }
    }

    fn description(self, language: Language) -> &'static str {
        match self {
            Self::Studio => language.text(Text::StudioDescription),
            Self::Recent => match language {
                Language::Chinese => "继续处理最近打开的标签文件。",
                Language::English => "Continue working on a recently opened label file.",
            },
            Self::Setup => language.text(Text::SetupDescription),
            Self::Settings => language.text(Text::SettingsDescription),
            Self::Updates => match language {
                Language::Chinese => "检查新版本，在准备好时安装。",
                Language::English => "Check for a new version and install when you are ready.",
            },
            Self::Jobs => language.text(Text::JobsDescription),
        }
    }
}

pub struct PrinterApp {
    studio: Entity<crate::workbench::Workbench>,
    page: Page,
    focus: FocusHandle,
    scroll: ScrollHandle,
    logo: Arc<Image>,
    font: Font,
    preferences: Preferences,
    preferences_path: PathBuf,
    updates: crate::updates::Updates,
    update_after_save: bool,
    snapshot: Option<Snapshot>,
    connection_error: Option<String>,
    output_dir: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    #[cfg(windows)]
    driver: Entity<SelectState<Vec<SharedString>>>,
    draft: Settings,
    commands: mpsc::Sender<Work>,
    notice: Option<Result<String, String>>,
    activity: Option<Activity>,
    modal: bool,
    jobs_filter: u8,
    jobs_search: Entity<InputState>,
    expanded_job: Option<String>,
}

impl PrinterApp {
    pub fn new(
        paths: AppPaths,
        font: Font,
        preferences: Preferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let jobs_search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter / 筛选"));
        cx.observe(&jobs_search, |_, _, cx| cx.notify()).detach();
        let preferences_path = paths.root.join("desktop.json");
        let draft = Settings::default();
        let output_dir = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(draft.output_dir.to_string_lossy().into_owned())
        });
        let width =
            cx.new(|cx| InputState::new(window, cx).default_value(draft.width_mm.to_string()));
        let height =
            cx.new(|cx| InputState::new(window, cx).default_value(draft.height_mm.to_string()));
        #[cfg(windows)]
        let driver = {
            let state = cx.new(|cx| SelectState::new(Vec::<SharedString>::new(), None, window, cx));
            cx.subscribe(
                &state,
                |this, _, event: &SelectEvent<Vec<SharedString>>, cx| {
                    let SelectEvent::Confirm(selected) = event;
                    this.draft.windows_driver = selected.as_ref().map(ToString::to_string);
                    this.sync_driver_resolution();
                    cx.notify();
                },
            )
            .detach();
            state
        };
        let studio = cx.new(|cx| {
            crate::workbench::Workbench::new(paths.clone(), preferences.language, window, cx)
        });
        // Root owns frame invalidation. Avoid extra immediate window refreshes
        // at every ancestor of a document notification.
        cx.observe(&studio, |s, _, cx| {
            if matches!(s.page, Page::Studio | Page::Recent) {
                cx.notify();
            }
        })
        .detach();
        let (updates, update_events) = crate::updates::Updates::new(paths.clone());
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = update_events.recv().await {
                if this
                    .update_in(cx, |s, w, cx| s.receive_update(event, w, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let (commands, events) = worker::spawn(paths);
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if cx
                    .update(|window, cx| this.update(cx, |app, cx| app.receive(event, window, cx)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let focus = cx.focus_handle();
        focus.focus(window);
        Self {
            studio,
            page: Page::Studio,
            focus,
            scroll: ScrollHandle::default(),
            logo: branding::logo(),
            font,
            preferences,
            preferences_path,
            updates,
            update_after_save: false,
            snapshot: None,
            connection_error: None,
            output_dir,
            width,
            height,
            #[cfg(windows)]
            driver,
            draft,
            commands,
            notice: None,
            activity: None,
            modal: false,
            jobs_filter: 0,
            jobs_search,
            expanded_job: None,
        }
    }

    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let close = self
            .studio
            .update(cx, |studio, cx| studio.request_close(window, cx));
        if !close {
            self.change_page(Page::Studio, window, cx);
        }
        close
    }
    pub fn open_document(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.change_page(Page::Studio, window, cx);
        self.studio
            .update(cx, |studio, cx| studio.open_path(path, window, cx));
    }

    fn t(&self, zh: &'static str, en: &'static str) -> &'static str {
        match self.preferences.language {
            Language::Chinese => zh,
            Language::English => en,
        }
    }

    fn text(&self, text: Text) -> &'static str {
        self.preferences.language.text(text)
    }

    fn switch_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preferences = Preferences {
            language: self.preferences.language.other(),
            ..self.preferences.clone()
        };
        match preferences.save(&self.preferences_path) {
            Ok(()) => {
                self.preferences = preferences;
                crate::apply_language(self.preferences.language, cx);
                self.studio.update(cx, |studio, cx| {
                    studio.set_language(self.preferences.language, cx);
                    cx.notify();
                });
                self.notice = None;
                window.refresh();
            }
            Err(error) => {
                self.notice = Some(Err(format!(
                    "{}\n{error}",
                    self.text(Text::SaveLanguageError)
                )));
            }
        }
        cx.notify();
    }

    fn receive(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Status(Ok(snapshot)) => {
                if self.connection_error.is_none() && self.snapshot.as_ref() == Some(&snapshot) {
                    return;
                }
                self.apply_snapshot(snapshot, false, window, cx);
            }
            Event::Status(Err(error)) => {
                if self.connection_error.as_ref() == Some(&error) {
                    return;
                }
                self.connection_error = Some(error);
            }
            Event::Completed {
                activity,
                updates_settings,
                result,
            } => {
                self.activity = None;
                self.notice = Some(result.map(|snapshot| {
                    self.apply_snapshot(snapshot, updates_settings, window, cx);
                    if activity == Activity::Test {
                        self.change_page(Page::Jobs, window, cx);
                    }
                    match activity {
                        Activity::Refresh => self.text(Text::EnvironmentRefreshed),
                        Activity::Save => self.text(Text::SettingsSaved),
                        Activity::Install => self.text(Text::PrinterConfigured),
                        Activity::Test => self.text(Text::TestSubmitted),
                        Activity::Retry => self.text(Text::JobRetried),
                        Activity::Remove => self.text(Text::PrinterRemoved),
                    }
                    .into()
                }));
                self.scroll.set_offset(Point::default());
                if activity == Activity::Save && self.update_after_save {
                    self.update_after_save = false;
                    if self.notice.as_ref().is_some_and(|n| n.is_ok()) {
                        self.confirm_update(window, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    fn apply_snapshot(
        &mut self,
        snapshot: Snapshot,
        sync_form: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.studio.update(cx, |studio, cx| {
            studio.configure(&snapshot.settings, window, cx)
        });
        let first = self.snapshot.is_none();
        if first && self.page == Page::Setup && snapshot.settings.setup_complete {
            self.change_page(Page::Jobs, window, cx);
        }
        if first || sync_form {
            self.draft = snapshot.settings.clone();
            for (input, value) in [
                (
                    &self.output_dir,
                    self.draft.output_dir.to_string_lossy().into_owned(),
                ),
                (&self.width, self.draft.width_mm.to_string()),
                (&self.height, self.draft.height_mm.to_string()),
            ] {
                if input.read(cx).value().as_str() != value {
                    input.update(cx, |state, cx| state.set_value(value, window, cx));
                }
            }
        }
        #[cfg(windows)]
        if sync_form
            || self.snapshot.as_ref().is_none_or(|previous| {
                previous.platform.windows_drivers != snapshot.platform.windows_drivers
            })
        {
            let drivers = &snapshot.platform.windows_drivers;
            if self
                .draft
                .windows_driver
                .as_ref()
                .is_none_or(|driver| !drivers.contains(driver))
            {
                self.draft.windows_driver = (drivers.len() == 1).then(|| drivers[0].clone());
            }
            let selected = self.draft.windows_driver.clone().unwrap_or_default();
            self.driver.update(cx, |state, cx| {
                state.set_items(
                    drivers
                        .iter()
                        .map(|driver| SharedString::from(driver.clone()))
                        .collect(),
                    window,
                    cx,
                );
                state.set_selected_value(&selected.into(), window, cx);
                cx.notify();
            });
            self.sync_driver_resolution();
        }
        self.snapshot = Some(snapshot);
        self.connection_error = None;
    }

    fn change_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            self.studio
                .update(cx, |s, cx| s.set_visible(page == Page::Studio, window, cx));
            self.focus.focus(window);
            self.scroll.set_offset(Point::default());
            self.notice = None;
            cx.notify();
        }
    }

    fn scroll_page(&self, direction: f32, cx: &mut Context<Self>) {
        let mut offset = self.scroll.offset();
        let page = self.scroll.bounds().size.height * 0.85;
        offset.y = (offset.y + page * direction).clamp(-self.scroll.max_offset().height, px(0.));
        self.scroll.set_offset(offset);
        cx.notify();
    }

    fn connected(&self) -> bool {
        self.snapshot.is_some() && self.connection_error.is_none()
    }

    fn queue_installed(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.settings.setup_complete && snapshot.platform.queue_installed
        })
    }

    fn busy(&self) -> bool {
        self.activity.is_some() || self.modal
    }

    fn send(&mut self, work: Work, cx: &mut Context<Self>) {
        if self.busy() || (!self.connected() && work.activity() != Activity::Refresh) {
            return;
        }
        let activity = work.activity();
        match self.commands.send(work) {
            Ok(()) => {
                self.notice = None;
                self.activity = Some(activity);
            }
            Err(_) => self.notice = Some(Err(self.text(Text::WorkerStopped).into())),
        }
        cx.notify();
    }

    #[cfg(windows)]
    fn driver_resolution(&self) -> Option<i32> {
        let name = self
            .draft
            .windows_driver
            .as_deref()?
            .to_ascii_lowercase()
            .replace(' ', "");
        if name.contains("203dpi") || name.contains("200dpi") {
            Some(8)
        } else if name.contains("300dpi") {
            Some(12)
        } else {
            None
        }
    }

    #[cfg(windows)]
    fn sync_driver_resolution(&mut self) {
        if let Some(dpmm) = self.driver_resolution() {
            self.draft.dpmm = dpmm;
        }
    }

    fn can_install(&self) -> bool {
        if self.busy() || !self.connected() {
            return false;
        }
        #[cfg(windows)]
        {
            self.draft.windows_driver.is_some()
                && self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.raw_listening)
        }
        #[cfg(not(windows))]
        true
    }

    fn submit_form(&mut self, activity: Activity, cx: &mut Context<Self>) {
        if self.busy()
            || !self.connected()
            || (activity == Activity::Install && !self.can_install())
        {
            return;
        }
        let snapshot = self
            .snapshot
            .as_ref()
            .expect("Connected app has a snapshot");
        let result = (|| -> anyhow::Result<Settings> {
            let mut settings = self.draft.clone();
            settings.output_dir = PathBuf::from(self.output_dir.read(cx).value().as_str());
            settings.width_mm = self
                .width
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!(self.text(Text::InvalidWidth)))?;
            settings.height_mm = self
                .height
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!(self.text(Text::InvalidHeight)))?;
            settings.raw_port = snapshot.settings.raw_port;
            settings.ipp_port = snapshot.settings.ipp_port;
            settings.setup_complete = snapshot.settings.setup_complete;
            anyhow::ensure!(
                settings.output_dir.is_absolute(),
                self.text(Text::InvalidDirectory)
            );
            labelize_printer_core::validate_media(settings.width_mm, settings.height_mm)
                .map_err(|_| anyhow::anyhow!(self.text(Text::InvalidMedia)))?;
            settings.validate()?;
            Ok(settings)
        })();
        match result {
            Ok(settings) => self.send(
                match activity {
                    Activity::Save => Work::Save(settings),
                    Activity::Install => Work::Install(settings),
                    Activity::Test => Work::Test(Some(settings)),
                    _ => unreachable!(),
                },
                cx,
            ),
            Err(error) => {
                self.notice = Some(Err(error.to_string()));
                self.scroll.set_offset(Point::default());
                cx.notify();
            }
        }
    }

    fn choose_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.modal = true;
        cx.notify();
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.text(Text::ChooseOutputFolder).into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |app, cx| {
                    app.modal = false;
                    match result {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.first() {
                                app.output_dir.update(cx, |input, cx| {
                                    input.set_value(
                                        path.to_string_lossy().into_owned(),
                                        window,
                                        cx,
                                    );
                                });
                                app.notice = None;
                            }
                        }
                        Ok(Err(error)) => {
                            app.notice = Some(Err(format!(
                                "{}\n{error}",
                                app.text(Text::FolderPickerError)
                            )));
                            app.scroll.set_offset(Point::default());
                        }
                        _ => {}
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }

    fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || !self.connected() || !self.queue_installed() {
            return;
        }
        self.modal = true;
        cx.notify();
        let choice = window.prompt(
            PromptLevel::Warning,
            self.text(Text::ConfirmRemove),
            Some(self.text(Text::ConfirmRemoveDescription)),
            &[
                PromptButton::cancel(self.text(Text::Cancel)),
                PromptButton::new(self.text(Text::RemoveQueue)),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let confirmed = choice.await == Ok(1);
            let _ = this.update(cx, |app, cx| {
                app.modal = false;
                if confirmed {
                    app.send(Work::Remove, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        let result = if !path.is_absolute() || !path.exists() {
            Err(self.text(Text::MissingPath).into())
        } else {
            open::that_detached(path)
                .map_err(|error| format!("{}\n{error}", self.text(Text::OpenPathError)))
        };
        if let Err(error) = result {
            self.notice = Some(Err(error));
            self.scroll.set_offset(Point::default());
            cx.notify();
        }
    }

    fn sidebar(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = div()
            .w(px(if compact { 56. } else { 188. }))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .p_2()
            .gap_2()
            .bg(rgb(0xf8faff))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .h(px(64.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .child(img(self.logo.clone()).size(px(32.)).flex_shrink_0())
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Labelize Studio"),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(MUTED))
                                        .child("LABEL WORKSPACE"),
                                ),
                        )
                    }),
            );
        for (index, page, icon) in [
            (0, Page::Studio, IconName::LayoutDashboard),
            (1, Page::Recent, IconName::FolderOpen),
            (2, Page::Jobs, IconName::Inbox),
        ] {
            nav = nav.child(
                Button::new(("nav", index as usize))
                    .ghost()
                    .w_full()
                    .justify_start()
                    .icon(icon)
                    .selected(self.page == page)
                    .tooltip(page.title(self.preferences.language))
                    .when(!compact, |b| b.label(page.title(self.preferences.language)))
                    .on_click(cx.listener(move |app, _, w, cx| app.change_page(page, w, cx))),
            );
        }
        nav = nav.child(div().flex_1());
        let ready = self.queue_installed() && self.connected();
        nav = nav
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .when(!compact, |d| d.bg(rgb(0xedf3fc)))
                    .child(div().size(px(7.)).rounded_full().bg(rgb(if ready {
                        0x25a675
                    } else {
                        0xa1aaba
                    })))
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(if ready {
                                    self.t("打印机已就绪", "Printer ready")
                                } else {
                                    self.t("打印机未就绪", "Printer not ready")
                                }),
                        )
                    }),
            )
            .child(
                Button::new("switch-language")
                    .ghost()
                    .w_full()
                    .justify_start()
                    .icon(IconName::Globe)
                    .tooltip(self.preferences.language.other().name())
                    .when(!compact, |b| {
                        b.label(self.preferences.language.other().name())
                    })
                    .disabled(self.modal)
                    .on_click(cx.listener(|s, _, w, cx| s.switch_language(w, cx))),
            )
            .child(
                Button::new("nav-settings")
                    .ghost()
                    .w_full()
                    .justify_start()
                    .icon(IconName::Settings2)
                    .tooltip(self.t("设置", "Settings"))
                    .selected(matches!(self.page, Page::Setup | Page::Settings))
                    .when(!compact, |b| b.label(self.t("设置", "Settings")))
                    .on_click(cx.listener(|app, _, w, cx| app.change_page(Page::Settings, w, cx))),
            );
        nav
    }

    fn recent_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let files = self.studio.read(cx).recent.clone();
        let mut list = div()
            .flex()
            .flex_col()
            .gap_1()
            .rounded_lg()
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(rgb(BORDER));
        if files.is_empty() {
            list = list.child(div().p_6().text_color(rgb(MUTED)).child(self.t(
                "打开文件后会在这里显示。",
                "Files you open will appear here.",
            )));
        }
        for (i, path) in files.into_iter().enumerate() {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            list = list.child(
                div()
                    .id(("recent", i))
                    .p_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(0xeef3ff)))
                    .child(Icon::new(IconName::File).text_color(rgb(ACCENT)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .truncate()
                                    .child(path.to_string_lossy().into_owned()),
                            ),
                    )
                    .child(Icon::new(IconName::ChevronRight).text_color(rgb(MUTED)))
                    .on_click(cx.listener(move |s, _, w, cx| s.open_document(path.clone(), w, cx))),
            );
        }
        list.into_any_element()
    }

    fn refresh_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("refresh-platform")
            .label(if self.activity == Some(Activity::Refresh) {
                self.text(Text::Detecting)
            } else if self.connected() {
                self.text(Text::Refresh)
            } else {
                self.text(Text::Connect)
            })
            .disabled(self.busy())
            .on_click(cx.listener(|app, _, _, cx| app.send(Work::Refresh, cx)))
    }

    fn form(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy() || self.snapshot.is_none();
        let format_row = [
            ("pdf", OutputFormat::Pdf, self.text(Text::PdfFormat)),
            ("png", OutputFormat::Png, self.text(Text::PngFormat)),
        ]
        .into_iter()
        .fold(button_row(), |row, (id, format, label)| {
            row.child(
                Button::new(id)
                    .outline()
                    .selected(self.draft.format == format)
                    .label(label)
                    .disabled(disabled)
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.draft.format = format;
                        cx.notify();
                    })),
            )
        });
        #[cfg(windows)]
        let fixed_resolution = self.driver_resolution();
        #[cfg(not(windows))]
        let fixed_resolution: Option<i32> = None;
        let dpi_row = [(8, "203 dpi / 8 dpmm"), (12, "300 dpi / 12 dpmm")]
            .into_iter()
            .fold(button_row(), |row, (dpmm, label)| {
                row.child(
                    Button::new(("dpmm", dpmm as usize))
                        .outline()
                        .selected(self.draft.dpmm == dpmm)
                        .label(label)
                        .disabled(disabled || fixed_resolution.is_some())
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.draft.dpmm = dpmm;
                            cx.notify();
                        })),
                )
            });
        let dir = div()
            .flex()
            .min_w_0()
            .gap_2()
            .when(compact, |this| this.flex_col().items_start())
            .when(!compact, |this| this.items_center())
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .when(!compact, |this| this.flex_1())
                    .child(Input::new(&self.output_dir).disabled(disabled)),
            )
            .child(
                Button::new("browse")
                    .label(self.text(Text::Browse))
                    .disabled(disabled)
                    .on_click(cx.listener(|app, _, window, cx| app.choose_directory(window, cx))),
            );
        card(compact)
            .child(section_title(self.text(Text::FilesAndLabels)))
            .child(field(self.text(Text::OutputFormat), format_row))
            .child(field(
                self.text(Text::RenderMode),
                button_row().children(
                    [
                        (RenderMode::Raster, Text::RasterMode),
                        (RenderMode::Vector, Text::VectorMode),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (mode, label))| {
                        Button::new(("default-render-mode", i))
                            .outline()
                            .label(self.text(label))
                            .selected(self.draft.render_mode == mode)
                            .disabled(disabled)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.draft.render_mode = mode;
                                cx.notify();
                            }))
                    }),
                ),
            ))
            .child(
                button_row().child(
                    Button::new("default-antialias")
                        .outline()
                        .selected(self.draft.antialias)
                        .disabled(disabled)
                        .label(self.text(if self.draft.antialias {
                            Text::AntialiasOn
                        } else {
                            Text::AntialiasOff
                        }))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.draft.antialias = !app.draft.antialias;
                            cx.notify();
                        })),
                ),
            )
            .when(
                self.draft.render_mode == RenderMode::Vector
                    && self.draft.format == OutputFormat::Png,
                |panel| {
                    panel.child(field(
                        self.text(Text::PngScale),
                        button_row().children((1..=4u32).map(|scale| {
                            Button::new(("default-png-scale", scale as usize))
                                .outline()
                                .label(format!("{scale}×"))
                                .selected(self.draft.png_scale == scale)
                                .disabled(disabled)
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.draft.png_scale = scale;
                                    cx.notify();
                                }))
                        })),
                    ))
                },
            )
            .child(div().text_sm().text_color(rgb(MUTED)).child(self.text(
                if self.draft.render_mode == RenderMode::Vector {
                    Text::VectorDescription
                } else {
                    Text::AntialiasDescription
                },
            )))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(self.text(Text::RenderDefaultsDescription)),
            )
            .child(field(self.text(Text::OutputFolder), dir))
            .child(field(
                self.text(Text::LabelSize),
                button_row()
                    .items_center()
                    .child(
                        div()
                            .w(px(100.))
                            .child(Input::new(&self.width).disabled(disabled)),
                    )
                    .child("×")
                    .child(
                        div()
                            .w(px(100.))
                            .child(Input::new(&self.height).disabled(disabled)),
                    ),
            ))
            .child(field(self.text(Text::ZplResolution), dpi_row))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(if fixed_resolution.is_some() {
                        self.text(Text::DriverResolution)
                    } else {
                        self.text(Text::MediaDescription)
                    }),
            )
            .child(
                button_row().child(
                    Button::new("autostart")
                        .outline()
                        .selected(self.draft.autostart)
                        .label(if self.draft.autostart {
                            self.text(Text::AutostartOn)
                        } else {
                            self.text(Text::AutostartOff)
                        })
                        .disabled(disabled)
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.draft.autostart = !app.draft.autostart;
                            cx.notify();
                        })),
                ),
            )
    }

    fn setup(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let backend = if self.connected() {
            self.text(Text::Connected)
        } else if self.connection_error.is_some() {
            self.text(Text::Disconnected)
        } else {
            self.text(Text::Connecting)
        };
        let queue = if self.queue_installed() {
            self.text(Text::QueueAdded)
        } else {
            self.text(Text::QueueMissing)
        };
        #[allow(unused_mut)]
        let mut status = card(compact)
            .child(section_title(self.text(Text::SystemPrinter)))
            .child(
                button_row()
                    .items_center()
                    .child(badge(
                        backend,
                        if self.connected() { 0x17603d } else { MUTED },
                    ))
                    .child(badge(queue, MUTED))
                    .child(self.refresh_button(cx)),
            );
        #[cfg(windows)]
        {
            let no_drivers = self
                .snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.platform.windows_drivers.is_empty());
            if no_drivers {
                status = status
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(self.text(Text::InstallDriver)),
                    )
                    .child(
                        button_row().child(
                            Button::new("download-driver")
                                .label(self.text(Text::DownloadDriver))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    if let Err(error) = open::that_detached(
                                        labelize_printer_core::platform::DRIVER_URL,
                                    ) {
                                        app.notice = Some(Err(format!(
                                            "{}\n{error}",
                                            app.text(Text::DriverDownloadError)
                                        )));
                                        cx.notify();
                                    }
                                })),
                        ),
                    );
            } else {
                status = status
                    .child(field(
                        self.text(Text::ZplDriver),
                        Select::new(&self.driver)
                            .placeholder(self.text(Text::SelectDriver))
                            .w_full()
                            .disabled(self.busy()),
                    ))
                    .child(div().text_sm().text_color(rgb(MUTED)).child(
                        if self.draft.windows_driver.is_some() && self.driver_resolution().is_none()
                        {
                            self.text(Text::ChooseResolution)
                        } else {
                            self.text(Text::DisableStatusCheck)
                        },
                    ));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .min_w_0()
            .child(status)
            .child(self.form(compact, cx))
    }

    fn settings(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .min_w_0()
            .child(self.form(compact, cx))
            .when(self.queue_installed(), |this| {
                this.child(
                    card(compact)
                        .child(section_title(self.text(Text::SystemQueue)))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child(self.text(Text::BackgroundPrinting)),
                        )
                        .child(
                            button_row().child(
                                Button::new("remove-queue")
                                    .label(self.text(Text::RemovePrinter))
                                    .disabled(self.busy() || !self.connected())
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.confirm_remove(window, cx)
                                    })),
                            ),
                        ),
                )
            })
    }

    fn jobs(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let jobs = self
            .snapshot
            .as_ref()
            .map_or(&[][..], |snapshot| snapshot.jobs.as_slice());
        let mut list = div().min_w_0().flex().flex_col().gap_3().child(
            button_row().child(
                Button::new("test-job")
                    .primary()
                    .label(if self.activity == Some(Activity::Test) {
                        self.text(Text::Submitting)
                    } else {
                        self.text(Text::TestZpl)
                    })
                    .disabled(self.busy() || !self.connected())
                    .on_click(cx.listener(|app, _, _, cx| app.send(Work::Test(None), cx))),
            ),
        );
        list = list.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(div().w(px(220.)).child(Input::new(&self.jobs_search)))
                .children(
                    [
                        (0, self.t("全部", "All")),
                        (1, self.t("进行中", "In progress")),
                        (2, self.t("已保存", "Saved")),
                        (3, self.t("失败", "Failed")),
                    ]
                    .into_iter()
                    .map(|(filter, label)| {
                        Button::new(("jobs-filter", filter as usize))
                            .small()
                            .ghost()
                            .label(label)
                            .selected(self.jobs_filter == filter)
                            .on_click(cx.listener(move |s, _, _, cx| {
                                s.jobs_filter = filter;
                                cx.notify();
                            }))
                    }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(self.t("筛选最近 100 条任务", "Filters the latest 100 jobs")),
                ),
        );
        if jobs.is_empty() {
            list = list.child(
                card(compact)
                    .child(section_title(self.text(Text::NoJobs)))
                    .child(
                        div()
                            .text_color(rgb(MUTED))
                            .child(if self.queue_installed() {
                                self.text(Text::PrintFirstJob)
                            } else {
                                self.text(Text::SetupFirstJob)
                            }),
                    ),
            );
        }
        let query = self.jobs_search.read(cx).value().to_lowercase();
        let mut shown = 0;
        for job in jobs {
            if !job.title.to_lowercase().contains(&query) && !job.id.to_lowercase().contains(&query)
            {
                continue;
            }
            if match self.jobs_filter {
                1 => matches!(job.state, JobState::Saved | JobState::Failed),
                2 => job.state != JobState::Saved,
                3 => job.state != JobState::Failed,
                _ => false,
            } {
                continue;
            }
            shown += 1;
            let (label, color) = match job.state {
                JobState::Receiving => (self.text(Text::Receiving), MUTED),
                JobState::Queued => (self.text(Text::Queued), MUTED),
                JobState::Processing => (self.text(Text::Processing), ACCENT),
                JobState::Saved => (self.text(Text::Saved), 0x17603d),
                JobState::Failed => (self.text(Text::Failed), 0xa22a34),
            };
            let source = if job.source == "test" {
                self.text(Text::TestSource)
            } else {
                match &job.input {
                    PrintJobInput::Zpl { .. } => "ZPL",
                    PrintJobInput::LabelFile {
                        language: DocumentKind::Design,
                        ..
                    } => self.text(Text::DesignSource),
                    PrintJobInput::LabelFile { language, .. } => language.name(),
                    PrintJobInput::RasterPages { .. } => self.text(Text::SystemPrint),
                }
            };
            let time = chrono::DateTime::parse_from_rfc3339(&job.created_at)
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|_| job.created_at.clone());
            let detail = if job.page_count == 0 {
                format!("{time} · {source}")
            } else {
                format!(
                    "{time} · {source} · {}",
                    self.preferences.language.page_count(job.page_count)
                )
            };
            let mut row = card(true)
                .p_3()
                .id(SharedString::from(job.id.clone()))
                .gap_2()
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(job.title.clone()),
                        )
                        .child(badge(label, color)),
                )
                .child(div().text_sm().text_color(rgb(MUTED)).child(detail));
            if self.expanded_job.as_deref() == Some(&job.id) {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(format!("ID: {}", job.id)),
                );
                for file in &job.output_files {
                    row = row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(file.to_string_lossy().into_owned()),
                    );
                }
            }
            if let Some(error) = &job.error {
                row = row.child(div().text_sm().text_color(rgb(color)).child(error.clone()));
            }
            let details_id = job.id.clone();
            let mut controls = button_row().child(
                Button::new("job-details")
                    .small()
                    .ghost()
                    .label(self.t("详情", "Details"))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.expanded_job = if s.expanded_job.as_ref() == Some(&details_id) {
                            None
                        } else {
                            Some(details_id.clone())
                        };
                        cx.notify();
                    })),
            );
            if let Some(file) = job.output_files.first() {
                let file = file.clone();
                if let Some(folder) = file.parent().map(Path::to_path_buf) {
                    controls = controls.child(
                        Button::new("folder")
                            .label(self.text(Text::OpenFolder))
                            .on_click(cx.listener(move |app, _, _, cx| app.open_path(&folder, cx))),
                    );
                }
                controls = controls.child(
                    Button::new("open")
                        .label(if job.output_files.len() > 1 {
                            self.text(Text::OpenFirstPage)
                        } else {
                            self.text(Text::OpenFile)
                        })
                        .on_click(cx.listener(move |app, _, _, cx| app.open_path(&file, cx))),
                );
            }
            if job.state == JobState::Failed && job.transport_complete {
                let id = job.id.clone();
                controls = controls.child(
                    Button::new("retry")
                        .label(self.text(Text::Retry))
                        .disabled(self.busy() || !self.connected())
                        .on_click(
                            cx.listener(move |app, _, _, cx| app.send(Work::Retry(id.clone()), cx)),
                        ),
                );
            }
            list = list.child(row.child(controls));
        }
        if shown == 0 && !jobs.is_empty() {
            list = list.child(
                div()
                    .p_6()
                    .text_color(rgb(MUTED))
                    .child(self.t("没有匹配的任务", "No matching jobs")),
            );
        }
        list
    }

    fn footer(&self, padding: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy() || !self.connected();
        let mut actions = button_row().justify_end();
        match self.page {
            Page::Setup => {
                actions = actions
                    .child(
                        Button::new("test-setup")
                            .label(if self.activity == Some(Activity::Test) {
                                self.text(Text::Submitting)
                            } else {
                                self.text(Text::SaveAndTest)
                            })
                            .disabled(disabled)
                            .on_click(
                                cx.listener(|app, _, _, cx| app.submit_form(Activity::Test, cx)),
                            ),
                    )
                    .child(
                        Button::new("install")
                            .primary()
                            .label(if self.activity == Some(Activity::Install) {
                                self.text(Text::Configuring)
                            } else if self.queue_installed() {
                                self.text(Text::RepairPrinter)
                            } else {
                                self.text(Text::AddPrinter)
                            })
                            .disabled(!self.can_install())
                            .on_click(
                                cx.listener(|app, _, _, cx| app.submit_form(Activity::Install, cx)),
                            ),
                    );
            }
            Page::Settings => {
                actions = actions
                    .child(
                        Button::new("open-output")
                            .label(self.text(Text::OpenOutputFolder))
                            .on_click(cx.listener(|app, _, _, cx| {
                                let path = PathBuf::from(app.output_dir.read(cx).value().as_str());
                                app.open_path(&path, cx);
                            })),
                    )
                    .child(
                        Button::new("save")
                            .primary()
                            .label(if self.activity == Some(Activity::Save) {
                                self.text(Text::Saving)
                            } else {
                                self.text(Text::SaveSettings)
                            })
                            .disabled(disabled)
                            .on_click(
                                cx.listener(|app, _, _, cx| app.submit_form(Activity::Save, cx)),
                            ),
                    );
            }
            Page::Studio | Page::Recent | Page::Jobs | Page::Updates => {}
        }
        div()
            .flex()
            .justify_center()
            .flex_shrink_0()
            .bg(rgb(0xffffff))
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .w_full()
                    .max_w(px(if self.page == Page::Studio {
                        1240.
                    } else {
                        900.
                    }))
                    .px(padding)
                    .py_3()
                    .child(actions),
            )
    }
}

fn button_row() -> Div {
    div().flex().flex_wrap().min_w_0().gap_2()
}

fn card(compact: bool) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_4()
        .p(if compact { px(14.) } else { px(20.) })
        .rounded(px(12.))
        .bg(rgb(0xffffff))
        .border_1()
        .border_color(rgb(BORDER))
}

fn section_title(text: &'static str) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text)
}

fn field(label: &'static str, content: impl IntoElement) -> Div {
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
        .child(content)
}

fn badge(text: &'static str, color: u32) -> Div {
    div()
        .flex_shrink_0()
        .px_2()
        .py_1()
        .rounded_full()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .bg(rgb(0xedf1f7))
        .child(text)
}

fn banner(text: String, error: bool) -> Div {
    div()
        .min_w_0()
        .p_3()
        .rounded_md()
        .text_sm()
        .bg(rgb(if error { 0xfbeaec } else { 0xe7f5ed }))
        .text_color(rgb(if error { 0xa22a34 } else { 0x17603d }))
        .child(text)
}

impl Render for PrinterApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(1000.);
        let narrow = window.viewport_size().width < px(700.);
        let mut content = div().flex_1().min_w_0().min_h_0().flex().flex_col();
        if self.page != Page::Updates
            && (self.updates.offer.is_some() || self.updates.recovery.is_some())
        {
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .bg(rgb(0xeef3ff))
                    .text_xs()
                    .child(div().flex_1().child(if self.updates.recovery.is_some() {
                        self.t(
                            "检查安装状态后恢复打印。",
                            "Verify the installation to resume printing.",
                        )
                    } else {
                        self.t("有新版本可用。", "A new version is available.")
                    }))
                    .child(
                        Button::new("view-update")
                            .small()
                            .ghost()
                            .label(self.t("查看更新", "View update"))
                            .on_click(
                                cx.listener(|s, _, w, cx| s.change_page(Page::Updates, w, cx)),
                            ),
                    ),
            );
        }
        if self.page == Page::Studio {
            if !self.queue_installed() && !self.studio.read(cx).setup_dismissed {
                content = content.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(rgb(0xeef3ff))
                        .text_xs()
                        .child(div().flex_1().min_w(px(130.)).child(self.t(
                            "也可以从系统打印窗口接收标签任务。",
                            "Receive label jobs from your system print dialog.",
                        )))
                        .child(
                            Button::new("setup-printer-hint")
                                .small()
                                .ghost()
                                .label(self.t("配置打印机", "Set up printer"))
                                .on_click(
                                    cx.listener(|s, _, w, cx| s.change_page(Page::Setup, w, cx)),
                                ),
                        )
                        .child(
                            Button::new("skip-setup-hint")
                                .small()
                                .ghost()
                                .label(self.t("稍后", "Later"))
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.studio.update(cx, |s, cx| s.dismiss_setup(cx));
                                })),
                        ),
                );
            }
            content = content.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.studio.clone()),
            );
        } else {
            let page_content = match self.page {
                Page::Recent => self.recent_files(cx),
                Page::Setup => self.setup(narrow, cx).into_any_element(),
                Page::Settings => self.settings(narrow, cx).into_any_element(),
                Page::Updates => self.update_page(narrow, cx).into_any_element(),
                Page::Jobs => self.jobs(narrow, cx).into_any_element(),
                Page::Studio => unreachable!(),
            };
            let mut body = div()
                .w_full()
                .max_w(px(1100.))
                .min_w_0()
                .p(if narrow { px(16.) } else { px(28.) })
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.page.title(self.preferences.language)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child(self.page.description(self.preferences.language)),
                        ),
                );
            if matches!(self.page, Page::Setup | Page::Settings | Page::Updates) {
                body = body.child(
                    button_row()
                        .child(
                            Button::new("settings-output-tab")
                                .ghost()
                                .selected(self.page == Page::Settings)
                                .label(self.t("输出默认值", "Output defaults"))
                                .on_click(
                                    cx.listener(|s, _, w, cx| s.change_page(Page::Settings, w, cx)),
                                ),
                        )
                        .child(
                            Button::new("settings-printer-tab")
                                .ghost()
                                .selected(self.page == Page::Setup)
                                .label(self.t("系统打印机", "System printer"))
                                .on_click(
                                    cx.listener(|s, _, w, cx| s.change_page(Page::Setup, w, cx)),
                                ),
                        )
                        .child(
                            Button::new("settings-update-tab")
                                .ghost()
                                .selected(self.page == Page::Updates)
                                .label(self.t("软件更新", "Software update"))
                                .on_click(
                                    cx.listener(|s, _, w, cx| s.change_page(Page::Updates, w, cx)),
                                ),
                        ),
                );
            }
            if let Some(notice) = &self.notice {
                body = body.child(match notice {
                    Ok(s) => banner(s.clone(), false),
                    Err(s) => banner(s.clone(), true),
                });
            }
            if self.connection_error.is_some() {
                body = body
                    .child(banner(self.text(Text::ReconnectDescription).into(), true))
                    .child(self.refresh_button(cx));
            }
            if self.page == Page::Setup {
                if let Some(snapshot) = &self.snapshot {
                    for note in snapshot
                        .platform
                        .notes
                        .iter()
                        .filter(|n| !n.starts_with("Install the official ZDesigner"))
                    {
                        body = body.child(banner(note.clone(), true));
                    }
                }
            }
            content = content.child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("page-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .justify_center()
                                    .child(body.child(page_content)),
                            ),
                    )
                    .vertical_scrollbar(&self.scroll),
            );
            if matches!(self.page, Page::Setup | Page::Settings) {
                content = content.child(self.footer(px(16.), cx));
            }
        }
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(SURFACE))
            .font(self.font.clone())
            .text_size(px(14.))
            .line_height(relative(1.45))
            .text_color(rgb(TEXT))
            .key_context("LabelizePrinter")
            .track_focus(&self.focus)
            .on_action(cx.listener(|s, _: &crate::Quit, w, cx| {
                if s.request_close(w, cx) {
                    cx.quit();
                }
            }))
            .on_action(cx.listener(|s, _: &ShowStudio, w, cx| s.change_page(Page::Studio, w, cx)))
            .on_action(cx.listener(|s, _: &ShowRecent, w, cx| s.change_page(Page::Recent, w, cx)))
            .on_action(cx.listener(|s, _: &OpenLabel, w, cx| {
                s.change_page(Page::Studio, w, cx);
                s.studio.update(cx, |s, cx| s.open(w, cx));
            }))
            .on_action(cx.listener(|s, _: &NewLabel, w, cx| {
                s.change_page(Page::Studio, w, cx);
                s.studio.update(cx, |s, cx| s.new_design(w, cx));
            }))
            .on_action(cx.listener(|s, _: &SaveLabel, w, cx| {
                s.studio.update(cx, |s, cx| s.save(w, cx));
            }))
            .on_action(cx.listener(|s, _: &SaveLabelAs, w, cx| {
                s.studio.update(cx, |s, cx| s.save_as(w, cx));
            }))
            .on_action(cx.listener(|s, _: &ExportLabel, w, cx| {
                s.studio.update(cx, |s, cx| s.export(w, cx));
            }))
            .on_action(cx.listener(|s, _: &CloseLabel, w, cx| {
                s.studio.update(cx, |s, cx| s.close_tab(w, cx));
            }))
            .on_action(cx.listener(|s, _: &NextLabel, w, cx| {
                s.studio.update(cx, |s, cx| s.cycle_tab(false, w, cx));
            }))
            .on_action(cx.listener(|s, _: &PreviousLabel, w, cx| {
                s.studio.update(cx, |s, cx| s.cycle_tab(true, w, cx));
            }))
            .on_action(cx.listener(|s, _: &UndoLabel, w, cx| {
                if s.page == Page::Studio {
                    s.studio.update(cx, |s, cx| s.history(false, w, cx));
                }
            }))
            .on_action(cx.listener(|s, _: &RedoLabel, w, cx| {
                if s.page == Page::Studio {
                    s.studio.update(cx, |s, cx| s.history(true, w, cx));
                }
            }))
            .on_action(cx.listener(|s, _: &ShowSetup, w, cx| s.change_page(Page::Setup, w, cx)))
            .on_action(
                cx.listener(|s, _: &ShowSettings, w, cx| s.change_page(Page::Settings, w, cx)),
            )
            .on_action(cx.listener(|s, _: &ShowJobs, w, cx| s.change_page(Page::Jobs, w, cx)))
            .on_action(cx.listener(|s, _: &ScrollPageUp, _, cx| s.scroll_page(1., cx)))
            .on_action(cx.listener(|s, _: &ScrollPageDown, _, cx| s.scroll_page(-1., cx)))
            .child(
                TitleBar::new()
                    .on_close_window(cx.listener(|s, _, w, cx| {
                        if s.request_close(w, cx) {
                            w.remove_window();
                        }
                    }))
                    .child(
                        div().flex().w_full().items_center().pr_3().child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child("Labelize Studio"),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.sidebar(compact, cx))
                    .child(content),
            )
    }
}
