use gpui::{App, Font, FontFallbacks, Image, ImageFormat};
use std::sync::Arc;

pub fn logo() -> Arc<Image> {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../platform/icon.png").to_vec(),
    ))
}

pub fn ui_font(cx: &App) -> Font {
    let installed = cx.text_system().all_font_names();
    let has_font = |name: &str| installed.iter().any(|font| font.eq_ignore_ascii_case(name));
    #[cfg(target_os = "windows")]
    let (families, cjk) = (
        ["Segoe UI", "Arial"],
        ["Microsoft YaHei UI", "Microsoft YaHei", "SimSun"],
    );
    #[cfg(target_os = "macos")]
    let (families, cjk) = (
        [".SystemUIFont", "Helvetica Neue"],
        ["PingFang SC", "Heiti SC", "Songti SC"],
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let (families, cjk) = (
        ["Noto Sans", "DejaVu Sans"],
        [
            "Noto Sans CJK SC",
            "Source Han Sans SC",
            "WenQuanYi Zen Hei",
        ],
    );
    let family = families
        .into_iter()
        .find(|name| has_font(name))
        .unwrap_or(".SystemUIFont");
    let fallbacks: Vec<_> = cjk
        .into_iter()
        .filter(|name| has_font(name))
        .map(str::to_owned)
        .collect();
    let mut font = gpui::font(family);
    if !fallbacks.is_empty() {
        font.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
    }
    font
}

#[cfg(windows)]
pub fn set_window_icon(window: &gpui::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
                WindowsAndMessaging::{
                    LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED,
                    SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, WM_SETICON,
                },
            },
        },
    };

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    // The handles come from this process's live window and its embedded icon resource.
    unsafe {
        let Ok(module) = GetModuleHandleW(PCWSTR::null()) else {
            return;
        };
        let dpi = GetDpiForWindow(hwnd);
        for (kind, width, height) in [
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            if let Ok(icon) = LoadImageW(
                Some(HINSTANCE(module.0)),
                PCWSTR(1 as *const u16),
                IMAGE_ICON,
                GetSystemMetricsForDpi(width, dpi),
                GetSystemMetricsForDpi(height, dpi),
                LR_SHARED,
            ) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}
