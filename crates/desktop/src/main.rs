#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod branding;
mod i18n;
mod studio;
mod studio_worker;
mod ui;
mod workbench;
mod worker;

use gpui::{prelude::*, *};
use gpui_component::{Root, TitleBar};
use i18n::{Language, Preferences, Text};
use labelize_printer_core::AppPaths;
#[cfg(target_os = "linux")]
use std::time::Duration;
use ui::{
    CloseLabel, ExportLabel, NewLabel, NextLabel, OpenLabel, PreviousLabel, PrinterApp, SaveLabel,
    SaveLabelAs, ScrollPageDown, ScrollPageUp, ShowJobs, ShowRecent, ShowSettings, ShowStudio,
};

actions!(labelize_printer, [Quit]);

fn apply_language(language: Language, cx: &mut App) {
    gpui_component::set_locale(language.locale());
    cx.set_menus(vec![Menu {
        name: "Labelize Printer".into(),
        items: vec![MenuItem::action(language.text(Text::Quit), Quit)],
    }]);
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
    let preferences = Preferences::load(&paths.root.join("desktop.json")).unwrap_or_else(|error| {
        log::warn!("{error:#}");
        Preferences::default()
    });
    let app = Application::new().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        gpui_component::init(cx);
        apply_language(preferences.language, cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Light, None, cx);
        let font = branding::ui_font(cx);
        let theme = gpui_component::Theme::global_mut(cx);
        theme.font_family = font.family.clone();
        theme.radius = px(8.);
        theme.radius_lg = px(12.);
        theme.scrollbar_show = gpui_component::scroll::ScrollbarShow::Always;
        theme.colors.background = rgb(ui::SURFACE).into();
        theme.colors.foreground = rgb(ui::TEXT).into();
        theme.colors.muted_foreground = rgb(ui::MUTED).into();
        theme.colors.border = rgb(ui::BORDER).into();
        theme.colors.input = rgb(ui::BORDER).into();
        theme.colors.title_bar = rgb(0xffffff).into();
        theme.colors.title_bar_border = rgb(0xffffff).into();
        theme.colors.secondary = rgb(0xf0f3f9).into();
        theme.colors.secondary_hover = rgb(0xe7edf6).into();
        theme.colors.secondary_active = rgb(0xe6edff).into();
        theme.colors.secondary_foreground = rgb(ui::TEXT).into();
        theme.colors.primary = rgb(ui::ACCENT).into();
        theme.colors.ring = rgb(ui::ACCENT).into();
        theme.colors.primary_hover = rgb(0x254fe0).into();
        theme.colors.primary_active = rgb(0x2045c8).into();
        theme.colors.primary_foreground = rgb(0xffffff).into();
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.bind_keys([
            KeyBinding::new(&format!("{modifier}-q"), Quit, None),
            KeyBinding::new(
                &format!("{modifier}-4"),
                ShowStudio,
                Some("LabelizePrinter"),
            ),
            KeyBinding::new(&format!("{modifier}-o"), OpenLabel, Some("LabelizePrinter")),
            KeyBinding::new(&format!("{modifier}-n"), NewLabel, Some("LabelizePrinter")),
            KeyBinding::new(&format!("{modifier}-s"), SaveLabel, Some("LabelizePrinter")),
            KeyBinding::new(
                &format!("{modifier}-1"),
                ShowStudio,
                Some("LabelizePrinter"),
            ),
            KeyBinding::new(
                &format!("{modifier}-shift-s"),
                SaveLabelAs,
                Some("LabelizePrinter"),
            ),
            KeyBinding::new(
                &format!("{modifier}-e"),
                ExportLabel,
                Some("LabelizePrinter"),
            ),
            KeyBinding::new(
                &format!("{modifier}-w"),
                CloseLabel,
                Some("LabelizePrinter"),
            ),
            KeyBinding::new("ctrl-tab", NextLabel, Some("LabelizePrinter")),
            KeyBinding::new("ctrl-shift-tab", PreviousLabel, Some("LabelizePrinter")),
            KeyBinding::new(
                &format!("{modifier}-,"),
                ShowSettings,
                Some("LabelizePrinter"),
            ),
            KeyBinding::new(
                &format!("{modifier}-2"),
                ShowRecent,
                Some("LabelizePrinter"),
            ),
            KeyBinding::new(&format!("{modifier}-3"), ShowJobs, Some("LabelizePrinter")),
            KeyBinding::new("pageup", ScrollPageUp, Some("LabelizePrinter && !Input")),
            KeyBinding::new(
                "pagedown",
                ScrollPageDown,
                Some("LabelizePrinter && !Input"),
            ),
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        // Wayland exposes outputs without designating a primary one.
        let display = cx
            .primary_display()
            .or_else(|| cx.displays().into_iter().next());
        let bounds = display
            .map(|display| display.bounds())
            .filter(|screen| screen.size.width > px(0.0) && screen.size.height > px(0.0))
            .map_or_else(
                || Bounds::new(point(px(0.0), px(0.0)), size(px(600.0), px(370.0))),
                |screen| {
                    // Leave room for the taskbar and window frame on small displays.
                    let initial_size = size(
                        px(1280.0).min(screen.size.width * 0.92),
                        px(820.0).min(screen.size.height * 0.85),
                    );
                    Bounds::centered_at(screen.center(), initial_size)
                },
            );
        // Development-only viewport override makes layout checks reproducible.
        #[cfg(debug_assertions)]
        let bounds = std::env::var("LABELIZE_PRINTER_WINDOW_SIZE")
            .ok()
            .and_then(|value| {
                value
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)))
            })
            .filter(|(w, h)| (480.0..=4000.0).contains(w) && (360.0..=3000.0).contains(h))
            .map_or(bounds, |(w, h)| {
                Bounds::centered_at(bounds.center(), size(px(w), px(h)))
            });
        let main_window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Labelize Printer".into()),
                        ..TitleBar::title_bar_options()
                    }),
                    window_decorations: Some(WindowDecorations::Client),
                    window_min_size: Some(size(
                        bounds.size.width.min(px(480.0)),
                        bounds.size.height.min(px(360.0)),
                    )),
                    app_id: Some("labelize-printer".into()),
                    ..Default::default()
                },
                |window, cx| {
                    #[cfg(windows)]
                    branding::set_window_icon(window);
                    let view = cx.new(|cx| PrinterApp::new(paths, font, preferences, window, cx));
                    let weak = view.downgrade();
                    window.on_window_should_close(cx, move |window, cx| {
                        weak.update(cx, |app, cx| app.request_close(window, cx))
                            .unwrap_or(true)
                    });
                    if let Some(path) = std::env::args_os()
                        .nth(1)
                        .map(std::path::PathBuf::from)
                        .filter(|p| p.is_file())
                    {
                        view.update(cx, |app, cx| app.open_document(path, window, cx));
                    }
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
