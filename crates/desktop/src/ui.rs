use crate::branding;
use crate::worker::{self, Activity, Event, Work};
use gpui::{prelude::*, *};
#[cfg(windows)]
use gpui_component::select::{Select, SelectEvent, SelectState};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    scroll::ScrollableElement,
    Disableable, Selectable,
};
use labelize_printer_core::{AppPaths, JobState, OutputFormat, Settings, Snapshot};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
};

actions!(
    labelize_printer,
    [
        ShowSetup,
        ShowSettings,
        ShowJobs,
        ScrollPageUp,
        ScrollPageDown
    ]
);

const ACCENT: u32 = 0x315efb;
const TEXT: u32 = 0x17243b;
const MUTED: u32 = 0x57657a;
const BORDER: u32 = 0xdde4ee;
const SURFACE: u32 = 0xf4f6fa;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Setup,
    Settings,
    Jobs,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Setup => "安装向导",
            Self::Settings => "输出设置",
            Self::Jobs => "最近任务",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Setup => "添加系统打印机，或先用一份 ZPL 测试输出。",
            Self::Settings => "选择文件格式、保存目录和缺省标签尺寸。",
            Self::Jobs => "查看任务状态，打开已保存的文件。",
        }
    }
}

pub struct PrinterApp {
    page: Page,
    focus: FocusHandle,
    scroll: ScrollHandle,
    logo: Arc<Image>,
    font: Font,
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
}

impl PrinterApp {
    pub fn new(paths: AppPaths, font: Font, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            page: Page::Setup,
            focus,
            scroll: ScrollHandle::default(),
            logo: branding::logo(),
            font,
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
        }
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
                        Activity::Refresh => "打印环境已刷新。",
                        Activity::Save => "设置已保存，将用于新任务。",
                        Activity::Install => {
                            "打印机已配置，可以从系统打印窗口选择 Labelize Printer。"
                        }
                        Activity::Test => "测试任务已提交。文件写入后会显示为「已保存」。",
                        Activity::Retry => "任务已重新排队。",
                        Activity::Remove => "系统打印队列已移除。",
                    }
                    .into()
                }));
                self.scroll.set_offset(Point::default());
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
            Err(_) => self.notice = Some(Err("后台连接线程已退出，请重新打开应用。".into())),
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
                .map_err(|_| anyhow::anyhow!("请输入有效的标签宽度。"))?;
            settings.height_mm = self
                .height
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("请输入有效的标签高度。"))?;
            settings.raw_port = snapshot.settings.raw_port;
            settings.ipp_port = snapshot.settings.ipp_port;
            settings.setup_complete = snapshot.settings.setup_complete;
            anyhow::ensure!(
                settings.output_dir.is_absolute(),
                "请选择一个完整的输出目录路径。"
            );
            labelize_printer_core::validate_media(settings.width_mm, settings.height_mm).map_err(
                |_| anyhow::anyhow!("标签宽度和高度必须为大于 0 且不超过 4000 的毫米数。"),
            )?;
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
            prompt: Some("选择输出目录".into()),
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
                            app.notice = Some(Err(format!("无法打开目录选择器：{error}")));
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
            "移除 Labelize Printer？",
            Some("系统打印窗口将移除这台打印机，已保存的文件会保留。"),
            &[PromptButton::cancel("取消"), PromptButton::new("移除队列")],
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
            Err("此文件或目录尚未创建，或已被移动。".into())
        } else {
            open::that_detached(path).map_err(|error| format!("无法打开：{error}"))
        };
        if let Err(error) = result {
            self.notice = Some(Err(error));
            self.scroll.set_offset(Point::default());
            cx.notify();
        }
    }

    fn header(&self, compact: bool, padding: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let logo = div()
            .flex()
            .items_center()
            .gap_2()
            .child(img(self.logo.clone()).size(px(38.)).flex_shrink_0())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child("Labelize Printer"),
                    )
                    .child(div().text_xs().text_color(rgb(MUTED)).child("本地标签打印")),
            );
        let mut nav = div().flex().flex_wrap().gap_1();
        for (index, page) in [Page::Setup, Page::Settings, Page::Jobs]
            .into_iter()
            .enumerate()
        {
            nav = nav.child(
                Button::new(("nav", index))
                    .ghost()
                    .selected(self.page == page)
                    .label(page.title())
                    .on_click(
                        cx.listener(move |app, _, window, cx| app.change_page(page, window, cx)),
                    ),
            );
        }
        div()
            .flex()
            .flex_shrink_0()
            .px(padding)
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0xffffff))
            .when(compact, |this| this.flex_col())
            .when(!compact, |this| this.items_center().justify_between())
            .child(logo)
            .child(nav)
    }

    fn refresh_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("refresh-platform")
            .label(if self.activity == Some(Activity::Refresh) {
                "检测中…"
            } else if self.connected() {
                "刷新检测"
            } else {
                "连接后台"
            })
            .disabled(self.busy())
            .on_click(cx.listener(|app, _, _, cx| app.send(Work::Refresh, cx)))
    }

    fn form(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy() || self.snapshot.is_none();
        let format_row = [
            ("pdf", OutputFormat::Pdf, "PDF · 每个任务一份"),
            ("png", OutputFormat::Png, "PNG · 每页一张"),
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
                    .label("选择目录…")
                    .disabled(disabled)
                    .on_click(cx.listener(|app, _, window, cx| app.choose_directory(window, cx))),
            );
        card(compact)
            .child(section_title("文件与标签"))
            .child(field("输出格式", format_row))
            .child(field("保存目录", dir))
            .child(field(
                "缺省宽度 × 高度（mm）",
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
            .child(field("ZPL 分辨率", dpi_row))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(if fixed_resolution.is_some() {
                        "分辨率随所选驱动调整。"
                    } else {
                        "ZPL 声明的尺寸优先。系统栅格保留原尺寸。"
                    }),
            )
            .child(
                button_row().child(
                    Button::new("autostart")
                        .outline()
                        .selected(self.draft.autostart)
                        .label(if self.draft.autostart {
                            "登录自动启动：开"
                        } else {
                            "登录自动启动：关"
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
            "后台已连接"
        } else if self.connection_error.is_some() {
            "后台未连接"
        } else {
            "正在连接后台…"
        };
        let queue = if self.queue_installed() {
            "系统打印机已添加"
        } else {
            "系统打印机尚未添加"
        };
        #[allow(unused_mut)]
        let mut status = card(compact).child(section_title("系统打印机")).child(
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
                            .child("安装官方 ZDesigner ZPL 驱动后，点击「刷新检测」。"),
                    )
                    .child(
                        button_row().child(
                            Button::new("download-driver")
                                .label("Zebra 驱动下载")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    if let Err(error) = open::that_detached(
                                        labelize_printer_core::platform::DRIVER_URL,
                                    ) {
                                        app.notice =
                                            Some(Err(format!("无法打开驱动下载页：{error}")));
                                        cx.notify();
                                    }
                                })),
                        ),
                    );
            } else {
                status = status
                    .child(field(
                        "ZDesigner ZPL 驱动",
                        Select::new(&self.driver)
                            .placeholder("选择驱动")
                            .w_full()
                            .disabled(self.busy()),
                    ))
                    .child(div().text_sm().text_color(rgb(MUTED)).child(
                        if self.draft.windows_driver.is_some() && self.driver_resolution().is_none()
                        {
                            "请在下方选择与驱动一致的 ZPL 分辨率。"
                        } else {
                            "在驱动设置中将 Status check type 设置为 Disabled。"
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
                        .child(section_title("系统队列"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child("关闭窗口后，后台仍可接收打印。"),
                        )
                        .child(
                            button_row().child(
                                Button::new("remove-queue")
                                    .label("移除打印机…")
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
                        "正在提交…"
                    } else {
                        "测试 ZPL 输出"
                    })
                    .disabled(self.busy() || !self.connected())
                    .on_click(cx.listener(|app, _, _, cx| app.send(Work::Test(None), cx))),
            ),
        );
        if jobs.is_empty() {
            list = list.child(
                card(compact).child(section_title("暂无任务")).child(
                    div()
                        .text_color(rgb(MUTED))
                        .child(if self.queue_installed() {
                            "从系统打印窗口选择 Labelize Printer，或提交一份测试标签。"
                        } else {
                            "先测试文件输出，或到安装向导添加系统打印机。"
                        }),
                ),
            );
        }
        for job in jobs {
            let (label, color) = match job.state {
                JobState::Receiving => ("接收中", MUTED),
                JobState::Queued => ("排队中", MUTED),
                JobState::Processing => ("处理中", ACCENT),
                JobState::Saved => ("已保存", 0x17603d),
                JobState::Failed => ("失败", 0xa22a34),
            };
            let source = if job.source.starts_with("ipp:") {
                "系统打印"
            } else if job.source == "test" {
                "测试 ZPL"
            } else {
                "ZPL"
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
                format!("{time} · {source} · {} 页", job.page_count)
            };
            let mut row = card(compact)
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
            if let Some(error) = &job.error {
                row = row.child(div().text_sm().text_color(rgb(color)).child(error.clone()));
            }
            let mut controls = button_row();
            if let Some(file) = job.output_files.first() {
                let file = file.clone();
                if let Some(folder) = file.parent().map(Path::to_path_buf) {
                    controls =
                        controls.child(Button::new("folder").label("打开目录").on_click(
                            cx.listener(move |app, _, _, cx| app.open_path(&folder, cx)),
                        ));
                }
                controls = controls.child(
                    Button::new("open")
                        .label(if job.output_files.len() > 1 {
                            "打开第 1 页"
                        } else {
                            "打开文件"
                        })
                        .on_click(cx.listener(move |app, _, _, cx| app.open_path(&file, cx))),
                );
            }
            if job.state == JobState::Failed && job.transport_complete {
                let id = job.id.clone();
                controls = controls.child(
                    Button::new("retry")
                        .label("重试")
                        .disabled(self.busy() || !self.connected())
                        .on_click(
                            cx.listener(move |app, _, _, cx| app.send(Work::Retry(id.clone()), cx)),
                        ),
                );
            }
            list = list.child(row.child(controls));
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
                                "正在提交…"
                            } else {
                                "保存并测试输出"
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
                                "正在配置…"
                            } else if self.queue_installed() {
                                "修复打印机"
                            } else {
                                "添加打印机"
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
                            .label("打开输出目录")
                            .on_click(cx.listener(|app, _, _, cx| {
                                let path = PathBuf::from(app.output_dir.read(cx).value().as_str());
                                app.open_path(&path, cx);
                            })),
                    )
                    .child(
                        Button::new("save")
                            .primary()
                            .label(if self.activity == Some(Activity::Save) {
                                "正在保存…"
                            } else {
                                "保存设置"
                            })
                            .disabled(disabled)
                            .on_click(
                                cx.listener(|app, _, _, cx| app.submit_form(Activity::Save, cx)),
                            ),
                    );
            }
            Page::Jobs => {}
        }
        div()
            .flex_shrink_0()
            .px(padding)
            .py_2()
            .bg(rgb(0xffffff))
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(actions)
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
        .rounded_lg()
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
        .rounded_md()
        .text_sm()
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
        let compact = window.viewport_size().width < px(670.);
        let padding = if compact { px(16.) } else { px(24.) };
        let content = match self.page {
            Page::Setup => self.setup(compact, cx).into_any_element(),
            Page::Settings => self.settings(compact, cx).into_any_element(),
            Page::Jobs => self.jobs(compact, cx).into_any_element(),
        };
        let mut body = div()
            .w_full()
            .max_w(px(900.))
            .min_w_0()
            .p(padding)
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
                            .font_weight(FontWeight::BOLD)
                            .child(self.page.title()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(self.page.description()),
                    ),
            );
        if let Some(notice) = &self.notice {
            body = body.child(match notice {
                Ok(text) => banner(text.clone(), false),
                Err(text) => banner(text.clone(), true),
            });
        }
        if self.connection_error.is_some() {
            body = body.child(banner("后台未连接。可重新连接后继续操作。".into(), true));
            if self.page != Page::Setup {
                body = body.child(button_row().child(self.refresh_button(cx)));
            }
        }
        if self.page == Page::Setup {
            if let Some(snapshot) = &self.snapshot {
                for note in snapshot
                    .platform
                    .notes
                    .iter()
                    .filter(|note| !note.starts_with("Install the official ZDesigner"))
                {
                    body = body.child(banner(note.clone(), true));
                }
            }
        }
        let content = div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
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
                            .child(body.child(content)),
                    ),
            )
            .vertical_scrollbar(&self.scroll);

        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(SURFACE))
            .font(self.font.clone())
            .text_base()
            .line_height(relative(1.45))
            .text_color(rgb(TEXT))
            .key_context("LabelizePrinter")
            .track_focus(&self.focus)
            .on_action(cx.listener(|app, _: &ShowSetup, window, cx| {
                app.change_page(Page::Setup, window, cx)
            }))
            .on_action(cx.listener(|app, _: &ShowSettings, window, cx| {
                app.change_page(Page::Settings, window, cx)
            }))
            .on_action(
                cx.listener(|app, _: &ShowJobs, window, cx| {
                    app.change_page(Page::Jobs, window, cx)
                }),
            )
            .on_action(cx.listener(|app, _: &ScrollPageUp, _, cx| app.scroll_page(1., cx)))
            .on_action(cx.listener(|app, _: &ScrollPageDown, _, cx| app.scroll_page(-1., cx)))
            .child(self.header(compact, padding, cx))
            .child(content)
            .when(self.page != Page::Jobs, |this| {
                this.child(self.footer(padding, cx))
            })
    }
}
