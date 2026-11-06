#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use async_channel::Sender;
use gpui::{prelude::*, *};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    ActiveTheme, Disableable, Root,
};
use labelize_printer_core::{ipc, platform, *};
use std::{path::PathBuf, time::Duration};

actions!(labelize_printer, [Quit]);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Setup,
    Settings,
    Jobs,
}

enum Message {
    Snapshot(Snapshot),
    Done(String),
    Error(String),
}

struct PrinterApp {
    paths: AppPaths,
    page: Page,
    snapshot: Option<Snapshot>,
    output_dir: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    draft: Settings,
    sender: Sender<Message>,
    notice: Option<(bool, String)>,
    busy: bool,
}

impl PrinterApp {
    fn new(paths: AppPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Store::open(paths.clone())
            .and_then(|s| s.settings())
            .unwrap_or_default();
        let output_dir = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.output_dir.to_string_lossy().to_string())
        });
        let width =
            cx.new(|cx| InputState::new(window, cx).default_value(settings.width_mm.to_string()));
        let height =
            cx.new(|cx| InputState::new(window, cx).default_value(settings.height_mm.to_string()));
        let (sender, messages) = async_channel::unbounded();
        let worker_sender = sender.clone();
        let worker_paths = paths.clone();
        std::thread::spawn(move || {
            if let Err(e) = platform::ensure_daemon(&worker_paths) {
                let _ = worker_sender.send_blocking(Message::Error(format!("后台启动失败：{e:#}")));
            }
            loop {
                match ipc::request(&worker_paths, Request::Status)
                    .and_then(Response::decode::<Snapshot>)
                {
                    Ok(snapshot) => {
                        if worker_sender
                            .send_blocking(Message::Snapshot(snapshot))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        if worker_sender
                            .send_blocking(Message::Error(format!("后台连接中断：{e:#}")))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        cx.spawn(async move |this, cx| {
            while let Ok(message) = messages.recv().await {
                if this
                    .update(cx, |this, cx| {
                        match message {
                            Message::Snapshot(snapshot) => this.snapshot = Some(snapshot),
                            Message::Done(text) => {
                                this.notice = Some((false, text));
                                this.busy = false;
                            }
                            Message::Error(text) => {
                                this.notice = Some((true, text));
                                this.busy = false;
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            page: if settings.setup_complete {
                Page::Jobs
            } else {
                Page::Setup
            },
            paths,
            snapshot: None,
            output_dir,
            width,
            height,
            draft: settings,
            sender,
            notice: None,
            busy: false,
        }
    }

    fn send(&mut self, request: Request, success: &'static str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        let paths = self.paths.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result =
                ipc::request(&paths, request).and_then(Response::decode::<serde_json::Value>);
            let message = match result {
                Ok(_) => Message::Done(success.into()),
                Err(e) => Message::Error(format!("{e:#}")),
            };
            let _ = sender.send_blocking(message);
        });
        cx.notify();
    }

    fn save(&mut self, install: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let result = (|| -> anyhow::Result<Settings> {
            let mut settings = self.draft.clone();
            settings.output_dir = PathBuf::from(self.output_dir.read(cx).value().as_str());
            settings.width_mm = self
                .width
                .read(cx)
                .value()
                .parse()
                .map_err(|_| anyhow::anyhow!("请输入有效的标签宽度"))?;
            settings.height_mm = self
                .height
                .read(cx)
                .value()
                .parse()
                .map_err(|_| anyhow::anyhow!("请输入有效的标签高度"))?;
            if let Some(snapshot) = &self.snapshot {
                settings.raw_port = snapshot.settings.raw_port;
                settings.ipp_port = snapshot.settings.ipp_port;
                settings.setup_complete = snapshot.settings.setup_complete;
            }
            settings.validate()?;
            Ok(settings)
        })();
        match result {
            Err(error) => self.notice = Some((true, error.to_string())),
            Ok(settings) => {
                self.draft = settings.clone();
                self.busy = true;
                self.notice = None;
                let paths = self.paths.clone();
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let result = (|| -> anyhow::Result<()> {
                        ipc::request(&paths, Request::SaveSettings { settings })?
                            .decode::<serde_json::Value>()?;
                        if install {
                            ipc::request(&paths, Request::Install)?
                                .decode::<serde_json::Value>()?;
                        }
                        Ok(())
                    })();
                    let _ = sender.send_blocking(match result {
                        Ok(()) => Message::Done(
                            if install {
                                "打印机已配置，可以从系统打印窗口选择 Labelize Printer。"
                            } else {
                                "设置已保存，新任务将使用这些设置。"
                            }
                            .into(),
                        ),
                        Err(e) => Message::Error(format!("{e:#}")),
                    });
                });
            }
        }
        cx.notify();
    }

    fn choose_directory(&self, window: &mut Window, cx: &mut Context<Self>) {
        let result = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择输出目录".into()),
        });
        let input = self.output_dir.clone();
        window
            .spawn(cx, async move |cx| {
                if let Ok(Ok(Some(paths))) = result.await {
                    if let Some(path) = paths.first() {
                        let value = path.to_string_lossy().to_string();
                        let _ = cx.update(|window, cx| {
                            input.update(cx, |input, cx| input.set_value(value, window, cx))
                        });
                    }
                }
            })
            .detach();
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut menu = div().flex().flex_col().gap_2();
        for (index, page, label) in [
            (0, Page::Setup, "安装向导"),
            (1, Page::Settings, "输出设置"),
            (2, Page::Jobs, "最近任务"),
        ] {
            let selected = self.page == page;
            menu = menu.child(
                div()
                    .id(("nav", index as usize))
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .cursor_pointer()
                    .bg(if selected {
                        rgb(0xe9efff)
                    } else {
                        rgb(0xf5f6f8)
                    })
                    .text_color(if selected {
                        rgb(0x315efb)
                    } else {
                        rgb(0x566171)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = page;
                        cx.notify();
                    }))
                    .child(label),
            );
        }
        div()
            .w(px(206.0))
            .h_full()
            .flex()
            .flex_col()
            .p_5()
            .gap_8()
            .bg(rgb(0xf5f6f8))
            .border_r_1()
            .border_color(rgb(0xe5e8ed))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(35.0))
                            .rounded_lg()
                            .bg(rgb(0x315efb))
                            .text_color(rgb(0xffffff))
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_weight(FontWeight::BOLD)
                            .child("L"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Labelize"))
                            .child(div().text_xs().text_color(rgb(0x7b8493)).child("PRINTER")),
                    ),
            )
            .child(menu)
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x7b8493))
                    .child("本地处理 · 文件自动保存"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x7b8493))
                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
            )
    }

    fn form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut format_row = div().flex().gap_2();
        for (id, format, label) in [
            ("pdf", OutputFormat::Pdf, "PDF · 每个任务一份"),
            ("png", OutputFormat::Png, "PNG · 每页一张"),
        ] {
            let button =
                Button::new(id)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.format = format;
                        cx.notify();
                    }));
            format_row = format_row.child(if self.draft.format == format {
                button.primary()
            } else {
                button
            });
        }
        let mut dpi_row = div().flex().gap_2();
        for (id, dpmm, label) in [
            ("dpi203", 8, "203 dpi / 8 dpmm"),
            ("dpi300", 12, "300 dpi / 12 dpmm"),
        ] {
            let button =
                Button::new(id)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.dpmm = dpmm;
                        cx.notify();
                    }));
            dpi_row = dpi_row.child(if self.draft.dpmm == dpmm {
                button.primary()
            } else {
                button
            });
        }
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(field("输出格式", format_row))
            .child(field(
                "保存目录",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.output_dir)))
                    .child(Button::new("browse").label("选择…").on_click(
                        cx.listener(|this, _, window, cx| this.choose_directory(window, cx)),
                    )),
            ))
            .child(field(
                "缺省标签尺寸（mm）",
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(div().w(px(130.0)).child(Input::new(&self.width)))
                    .child("×")
                    .child(div().w(px(130.0)).child(Input::new(&self.height))),
            ))
            .child(field("ZPL 队列分辨率", dpi_row))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x7b8493))
                    .child("任务中的纸张尺寸优先。系统栅格保留原来的页面尺寸与分辨率。"),
            )
            .child(
                Button::new("autostart")
                    .label(if self.draft.autostart {
                        "✓ 登录后自动启动后台"
                    } else {
                        "登录后自动启动后台"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.draft.autostart = !this.draft.autostart;
                        cx.notify();
                    })),
            )
    }

    fn setup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.snapshot.as_ref().is_some_and(|s| s.raw_listening);
        let registered = self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.platform.queue_installed);
        #[allow(unused_mut)]
        let mut card =
            div()
                .flex()
                .flex_col()
                .gap_5()
                .p_6()
                .rounded_xl()
                .border_1()
                .border_color(rgb(0xe5e8ed))
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("准备你的虚拟打印机"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x687386))
                        .child("配置输出位置，然后将 Labelize Printer 添加到系统打印窗口。"),
                )
                .child(
                    div()
                        .flex()
                        .gap_6()
                        .text_sm()
                        .child(format!(
                            "后台：{}",
                            if ready { "运行中" } else { "等待连接" }
                        ))
                        .child(Button::new("refresh-platform").label("刷新检测").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.send(Request::RefreshPlatform, "打印环境已刷新。", cx)
                            }),
                        ))
                        .child(format!(
                            "打印机：{}",
                            if registered {
                                "已添加"
                            } else {
                                "尚未添加"
                            }
                        )),
                )
                .child(self.form(cx));
        #[cfg(windows)]
        {
            if let Some(snapshot) = &self.snapshot {
                let mut drivers = div().flex().flex_col().gap_2();
                for (i, name) in snapshot.platform.windows_drivers.iter().enumerate() {
                    let chosen = name.clone();
                    let label = if self.draft.windows_driver.as_ref() == Some(name) {
                        format!("✓ {name}")
                    } else {
                        name.clone()
                    };
                    drivers = drivers.child(Button::new(("driver", i)).label(label).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.draft.windows_driver = Some(chosen.clone());
                            cx.notify();
                        }),
                    ));
                }
                card = card.child(field("ZDesigner ZPL 驱动", drivers))
                    .child(Button::new("download-driver").label("打开 Zebra 官方驱动下载页").on_click(|_,_,_| { let _ = open::that_detached(platform::DRIVER_URL); }))
                    .child(div().text_xs().child("安装后请在 ZDesigner Driver Settings 中，将 Status check type 设置为 Disabled。驱动 dpi 应与上方配置一致。"));
            }
        }
        card.child(
            div()
                .flex()
                .gap_3()
                .child(
                    Button::new("install")
                        .primary()
                        .label(if self.busy {
                            "正在配置…"
                        } else {
                            "添加 / 修复打印机"
                        })
                        .disabled(self.busy || !ready)
                        .on_click(cx.listener(|this, _, _, cx| this.save(true, cx))),
                )
                .child(
                    Button::new("test-setup")
                        .label("测试 ZPL 输出")
                        .disabled(self.busy || !ready)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.send(
                                Request::TestPrint,
                                "测试任务已提交，请在最近任务中查看结果。",
                                cx,
                            )
                        })),
                ),
        )
    }

    fn settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .rounded_xl()
            .border_1()
            .border_color(rgb(0xe5e8ed))
            .child(self.form(cx))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("save")
                            .primary()
                            .label("保存设置")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.save(false, cx))),
                    )
                    .child(
                        Button::new("open-output")
                            .label("打开输出目录")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let path = PathBuf::from(this.output_dir.read(cx).value().as_str());
                                let _ = open::that_detached(path);
                            })),
                    ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(0xe5e8ed))
                    .pt_4()
                    .text_sm()
                    .text_color(rgb(0x687386))
                    .child("关闭此窗口后，后台继续接收打印。卸载队列不会删除已经生成的文件。"),
            )
            .child(
                Button::new("remove-queue")
                    .label("移除系统打印队列")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.send(
                            Request::Uninstall,
                            "系统打印队列已移除，输出文件已保留。",
                            cx,
                        )
                    })),
            )
    }

    fn jobs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let jobs = self
            .snapshot
            .as_ref()
            .map(|s| s.jobs.as_slice())
            .unwrap_or(&[]);
        let mut list = div().flex().flex_col().gap_3();
        if jobs.is_empty() {
            list = list.child(
                div()
                    .p_10()
                    .rounded_xl()
                    .bg(rgb(0xf7f8fa))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("等待第一份打印任务"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x687386))
                            .child("在任意应用的打印窗口中，选择 Labelize Printer。"),
                    )
                    .child(
                        Button::new("test-empty")
                            .label("先测试 ZPL 输出")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.send(Request::TestPrint, "测试任务已提交。", cx)
                            })),
                    ),
            );
        }
        for (i, job) in jobs.iter().enumerate() {
            let (label, color) = match job.state {
                JobState::Receiving => ("接收中", 0x687386),
                JobState::Queued => ("排队中", 0x687386),
                JobState::Processing => ("处理中", 0x315efb),
                JobState::Saved => ("已保存", 0x237753),
                JobState::Failed => ("失败", 0xc44242),
            };
            let mut row = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0xe5e8ed))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(job.title.clone()),
                        )
                        .child(div().text_sm().text_color(rgb(color)).child(label)),
                )
                .child(div().text_xs().text_color(rgb(0x7b8493)).child(format!(
                        "{}  ·  {}  ·  {} 页",
                        chrono::DateTime::parse_from_rfc3339(&job.created_at)
                            .map(|time| time
                                .with_timezone(&chrono::Local)
                                .format("%m-%d %H:%M:%S")
                                .to_string())
                            .unwrap_or_else(|_| job.created_at.clone()),
                        if job.source.starts_with("ipp:") {
                            "系统打印"
                        } else {
                            "ZPL"
                        },
                        job.page_count
                    )));
            if let Some(error) = &job.error {
                row = row.child(
                    div()
                        .text_sm()
                        .text_color(rgb(0xc44242))
                        .child(error.clone()),
                );
            }
            let mut actions = div().flex().gap_2();
            if let Some(file) = job.output_files.first() {
                let file = file.clone();
                let folder = file.parent().unwrap().to_path_buf();
                actions =
                    actions
                        .child(Button::new(("open", i)).label("打开文件").on_click(
                            move |_, _, _| {
                                let _ = open::that_detached(&file);
                            },
                        ))
                        .child(Button::new(("folder", i)).label("打开目录").on_click(
                            move |_, _, _| {
                                let _ = open::that_detached(&folder);
                            },
                        ));
            }
            if job.state == JobState::Failed && job.transport_complete {
                let id = job.id.clone();
                actions = actions.child(
                    Button::new(("retry", i))
                        .label("重试")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.send(Request::Retry { id: id.clone() }, "任务已重新排队。", cx)
                        })),
                );
            }
            list = list.child(row.child(actions));
        }
        list
    }
}

fn field(label: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
        .child(content)
}

impl Render for PrinterApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, subtitle) = match self.page {
            Page::Setup => ("安装向导", "一次配置，在系统打印窗口中直接保存标签。"),
            Page::Settings => ("输出设置", "选择格式与目录，文件按打印任务自动整理。"),
            Page::Jobs => ("最近任务", "查看接收、转换与文件保存的实际结果。"),
        };
        let content = match self.page {
            Page::Setup => self.setup(cx).into_any_element(),
            Page::Settings => self.settings(cx).into_any_element(),
            Page::Jobs => self.jobs(cx).into_any_element(),
        };
        let mut main = div()
            .id("main-content")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .p_8()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_2xl().font_weight(FontWeight::BOLD).child(title))
                    .child(div().text_sm().text_color(rgb(0x7b8493)).child(subtitle)),
            );
        if let Some((error, text)) = &self.notice {
            main = main.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .text_sm()
                    .bg(if *error { rgb(0xffeeee) } else { rgb(0xeaf6ef) })
                    .text_color(if *error { rgb(0xa93636) } else { rgb(0x237753) })
                    .child(text.clone()),
            );
        }
        if let Some(snapshot) = &self.snapshot {
            for note in &snapshot.platform.notes {
                main = main.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .text_sm()
                        .bg(rgb(0xfff7e7))
                        .text_color(rgb(0x886323))
                        .child(note.clone()),
                );
            }
        }
        div()
            .flex()
            .size_full()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x1e293b))
            .text_base()
            .font_family(cx.theme().font_family.clone())
            .child(self.sidebar(cx))
            .child(main.child(content))
    }
}

struct DesktopLogger;
impl log::Log for DesktopLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }
    fn log(&self, record: &log::Record<'_>) {
        use std::io::Write;
        if self.enabled(record.metadata()) {
            let _ = writeln!(
                std::io::stderr().lock(),
                "{} {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }
    fn flush(&self) {}
}

fn main() -> anyhow::Result<()> {
    let _ = log::set_logger(&DesktopLogger);
    log::set_max_level(if std::env::var_os("LABELIZE_PRINTER_DEBUG").is_some() {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Warn
    });
    let paths = AppPaths::discover()?;
    Application::new().run(move |cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Light, None, cx);
        let theme = gpui_component::Theme::global_mut(cx);
        theme.colors.primary = rgb(0x315efb).into();
        theme.colors.primary_hover = rgb(0x254fe0).into();
        theme.colors.primary_active = rgb(0x2045c8).into();
        theme.colors.primary_foreground = rgb(0xffffff).into();
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            },
            Quit,
            None,
        )]);
        cx.set_menus(vec![Menu {
            name: "Labelize Printer".into(),
            items: vec![MenuItem::action("退出", Quit)],
        }]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1080.0), px(820.0)), cx);
        let main_window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Labelize Printer".into()),
                        ..Default::default()
                    }),
                    window_min_size: Some(size(px(800.0), px(600.0))),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| PrinterApp::new(paths, window, cx));
                    // Root owns an erased AnyView. Explicitly invalidate the window
                    // when its child model changes, including IPC updates.
                    cx.observe(&view, |_, cx| cx.refresh_windows()).detach();
                    cx.activate(true);
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Unable to open Labelize Printer window");
        #[cfg(target_os = "linux")]
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            // GPUI 0.2.2's X11 swapchain can retain the pre-map drawable until
            // its first configure event. Reconfigure once after mapping, then
            // restore the requested size. No user resize is needed.
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let original = main_window.update(cx, |_, window, _| {
                    let original = window.viewport_size();
                    window.resize(size(original.width + px(1.0), original.height));
                    original
                });
                if let Ok(original) = original {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let _ = main_window.update(cx, |_, window, _| window.resize(original));
                }
            })
            .detach();
        }
        #[cfg(not(target_os = "linux"))]
        let _ = main_window;
    });
    Ok(())
}
