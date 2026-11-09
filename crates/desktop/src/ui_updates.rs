use super::*;
use crate::updates::{Phase, UpdateEvent};
use labelize_printer_updater::{self as updater, handoff, InstallSource};
use std::sync::atomic::Ordering;

impl PrinterApp {
    pub(super) fn receive_update(
        &mut self,
        event: UpdateEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            UpdateEvent::Detected(source, target, manual) => {
                self.updates.source = self.preferences.update_source.clone().unwrap_or(source);
                self.updates.manual_install = manual;
                match target {
                    Ok(t) => self.updates.target = Some(t),
                    Err(e) => self.updates.error = Some(e),
                }
                if updater::check_due(
                    self.preferences.automatic_updates,
                    self.preferences.last_update_check,
                    chrono::Utc::now().timestamp(),
                ) {
                    self.check_update(false, cx);
                }
            }
            UpdateEvent::Checked(result, _manual) => {
                self.updates.phase = Phase::Idle;
                match result {
                    Ok(offer) => {
                        // Invalidate a cached installer if the release was replaced or withdrawn.
                        if self.updates.verified.as_ref().is_some_and(|v| {
                            offer.as_ref().is_none_or(|o| {
                                o.version != v.offer.version
                                    || o.installer.digest != v.offer.installer.digest
                                    || o.installer.size != v.offer.installer.size
                                    || o.sums.digest != v.offer.sums.digest
                            })
                        }) {
                            self.updates.verified = None;
                        }
                        self.updates.offer = offer;
                        self.updates.checked = true;
                        self.updates.error = None;
                    }
                    Err(e) => {
                        log::warn!("Update check: {e}");
                        self.updates.error = Some(e);
                        // A delayed check must not navigate away from a document the user is editing.
                    }
                }
            }
            UpdateEvent::Progress(done, total) => self.updates.progress = (done, total),
            UpdateEvent::Downloaded(result) => {
                self.updates.phase = Phase::Idle;
                match result {
                    Ok(file) => {
                        self.updates.verified = Some(file);
                        self.updates.error = None;
                    }
                    Err(e) => {
                        if self.updates.cancel.load(Ordering::Acquire) {
                            self.updates.message =
                                Some(self.t("下载已取消。", "Download cancelled.").into());
                        } else {
                            self.updates.error = Some(e);
                        }
                    }
                }
            }
            UpdateEvent::Prepared(result) => {
                self.modal = false;
                match result {
                    Ok(ticket) => {
                        self.updates.ticket = Some(ticket.clone());
                        // Re-save here: the user could have edited while verification ran.
                        if self.request_close(window, cx) {
                            match handoff::authorize(&ticket) {
                                Ok(()) => {
                                    cx.quit();
                                    return;
                                }
                                Err(error) => self.updates.error = Some(format!("{error:#}")),
                            }
                        }
                        handoff::cancel(&ticket);
                    }
                    Err(e) => self.updates.error = Some(e),
                }
                self.updates.ticket = None;
                self.updates.phase = Phase::Idle;
                self.studio
                    .update(cx, |s, cx| s.resume_update_previews(window, cx));
            }
            UpdateEvent::Pending(error) => {
                self.updates.recovery = Some(error);
                self.change_page(Page::Updates, window, cx);
            }
            UpdateEvent::Recovery(result) => {
                self.updates.phase = Phase::Idle;
                match result {
                    Ok(message) => {
                        self.updates.recovery = None;
                        self.updates.error = None;
                        self.updates.message = Some(message);
                        self.send(Work::Refresh, cx);
                    }
                    Err(e) => self.updates.recovery = Some(e),
                }
            }
        }
        cx.notify();
    }
    pub(super) fn check_update(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.updates.phase != Phase::Idle || self.updates.target.is_none() {
            return;
        }
        let mut prefs = self.preferences.clone();
        prefs.last_update_check = Some(chrono::Utc::now().timestamp());
        match prefs.save(&self.preferences_path) {
            Ok(()) => {
                self.preferences = prefs;
                self.updates.check(manual);
            }
            Err(e) => self.updates.error = Some(format!("{e:#}")),
        }
        cx.notify();
    }
    fn settings_dirty(&self, cx: &App) -> bool {
        self.snapshot.as_ref().is_some_and(|s| {
            self.draft != s.settings
                || self.output_dir.read(cx).value().as_str()
                    != s.settings.output_dir.to_string_lossy()
                || self.width.read(cx).value().trim().parse::<f64>().ok()
                    != Some(s.settings.width_mm)
                || self.height.read(cx).value().trim().parse::<f64>().ok()
                    != Some(s.settings.height_mm)
        })
    }
    pub(super) fn confirm_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.phase != Phase::Idle || self.modal || self.activity.is_some() {
            return;
        }
        if self.settings_dirty(cx) {
            self.modal = true;
            let prompt = window.prompt(
                PromptLevel::Warning,
                self.t("保存输出设置？", "Save output settings?"),
                Some(self.t(
                    "安装前请选择如何处理未保存的设置。标签草稿会自动保留。",
                    "Choose what to do with unsaved settings. Label drafts will be retained.",
                )),
                &[
                    PromptButton::cancel(self.t("取消", "Cancel")),
                    PromptButton::new(self.t("放弃设置更改", "Discard settings changes")),
                    PromptButton::new(self.t("保存并继续", "Save and continue")),
                ],
                cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                let choice = prompt.await.ok();
                let _ = this.update_in(cx, |s, w, cx| {
                    s.modal = false;
                    match choice {
                        Some(1) => s.begin_update(w, cx),
                        Some(2) => {
                            s.submit_form(Activity::Save, cx);
                            s.update_after_save = s.activity == Some(Activity::Save);
                        }
                        _ => cx.notify(),
                    }
                });
            })
            .detach();
            return;
        }
        self.begin_update(window, cx);
    }
    fn begin_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.updates.offer.as_ref() else {
            return;
        };
        if self.updates.source == InstallSource::Unknown {
            return;
        }
        if self.updates.source == InstallSource::Package && self.updates.verified.is_none() {
            return;
        }
        let plan = handoff::Plan {
            source: self.updates.source.clone(),
            version: offer.version.clone(),
            installer: if self.updates.source == InstallSource::Package {
                self.updates.verified.clone()
            } else {
                None
            },
            manual: self.updates.manual_install || self.updates.source == InstallSource::Homebrew,
        };
        if !self.request_close(window, cx) {
            return;
        }
        let command = if plan.source == InstallSource::Homebrew {
            Some(updater::BREW_COMMAND.to_string())
        } else if plan.manual {
            plan.installer
                .as_ref()
                .map(|v| updater::platform::apt_command(&v.path))
        } else {
            None
        };
        let detail = if let Some(command) = &command {
            format!("{}\n\n{command}\n\n{}", self.t("应用将退出并暂停打印。升级命令会复制到剪贴板，请在终端中执行。", "The app will close and pause printing. The upgrade command will be copied; run it in your terminal."),
                self.t("完成或取消后，重新打开 Labelize Studio。", "Reopen Labelize Studio after completing or cancelling installation."))
        } else {
            self.t("标签草稿将保留，打印暂时停止。请在系统安装器中完成安装，然后重新打开应用。试用安装包尚未签名。", "Drafts will be retained and printing paused. Complete the system installer, then reopen the app. Trial installers are unsigned.").into()
        };
        self.modal = true;
        let prompt = window.prompt(
            PromptLevel::Info,
            self.t("准备安装更新", "Ready to install update"),
            Some(&detail),
            &[
                PromptButton::cancel(self.t("取消", "Cancel")),
                PromptButton::new(self.t("退出并继续", "Quit and continue")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = prompt.await.ok();
            let _ = this.update_in(cx, |s, _w, cx| {
                s.modal = false;
                if choice != Some(1) {
                    cx.notify();
                    return;
                }
                if let Some(command) = command {
                    cx.write_to_clipboard(ClipboardItem::new_string(command));
                }
                s.updates.phase = Phase::Preparing;
                s.updates.error = None;
                s.modal = true;
                let queue = s.studio.read(cx).update_previews();
                queue.pause();
                let paths = s.updates.paths.clone();
                let tx = s.updates.events.clone();
                std::thread::spawn(move || {
                    let result = (|| -> anyhow::Result<handoff::Ticket> {
                        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
                        while !queue.idle() {
                            anyhow::ensure!(
                                std::time::Instant::now() < until,
                                "Preview renderer has not stopped; installation was not started"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                        handoff::prepare(&paths, plan)
                    })()
                    .map_err(|e| format!("{e:#}"));
                    let _ = tx.send_blocking(UpdateEvent::Prepared(result));
                });
                cx.notify();
            });
        })
        .detach();
    }
    fn set_update_source(&mut self, source: InstallSource, cx: &mut Context<Self>) {
        let mut prefs = self.preferences.clone();
        prefs.update_source = Some(source.clone());
        match prefs.save(&self.preferences_path) {
            Ok(()) => {
                self.preferences = prefs;
                self.updates.source = source;
            }
            Err(e) => self.updates.error = Some(format!("{e:#}")),
        }
        cx.notify();
    }
    pub(super) fn update_page(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let idle = self.updates.phase == Phase::Idle;
        let mut page = div().flex().flex_col().gap_4().min_w_0();
        let status = if self.updates.phase == Phase::Checking {
            self.t("正在检查…", "Checking…")
        } else if self.updates.phase == Phase::Preparing {
            self.t("正在准备安全交接…", "Preparing installation…")
        } else if self.updates.phase == Phase::Downloading {
            self.t("正在下载并校验…", "Downloading and verifying…")
        } else if self.updates.verified.is_some() {
            self.t("安装包已校验", "Installer verified")
        } else if self.updates.offer.is_some() {
            self.t("新版本可用", "Update available")
        } else if self.updates.error.is_some() {
            self.t("检查或下载未完成", "Check or download did not finish")
        } else if self.updates.checked {
            self.t("已是最新稳定版", "You have the latest stable version")
        } else {
            self.t("检查稳定版更新", "Check for stable updates")
        };
        let last = self
            .preferences
            .last_update_check
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| self.t("尚未检查", "Not checked yet").into());
        page = page.child(card(compact)
            .child(section_title(status))
            .child(div().text_sm().child(format!("{} {}", self.t("当前版本", "Current version"), env!("CARGO_PKG_VERSION"))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(format!("{}: {last}", self.t("上次检查", "Last check"))))
            .child(button_row()
                .child(Button::new("check-update").label(self.t("检查更新", "Check for updates")).disabled(!idle || self.updates.target.is_none()).on_click(cx.listener(|s, _, _, cx| s.check_update(true, cx))))
                .child(Button::new("automatic-updates").ghost().selected(self.preferences.automatic_updates)
                    .label(if self.preferences.automatic_updates { self.t("自动检查：开启", "Automatic checks: on") } else { self.t("自动检查：关闭", "Automatic checks: off") })
                    .on_click(cx.listener(|s, _, _, cx| {
                        let mut prefs = s.preferences.clone(); prefs.automatic_updates = !prefs.automatic_updates;
                        match prefs.save(&s.preferences_path) { Ok(()) => s.preferences = prefs, Err(e) => s.updates.error = Some(format!("{e:#}")) } cx.notify();
                    }))))
            .child(div().text_xs().text_color(rgb(MUTED)).child(self.t("启动时每天最多检查一次。下载和安装都由你确认。", "Checks at startup, at most once a day. You choose when to download and install."))));
        if let Some(error) = &self.updates.recovery {
            page = page.child(card(compact).child(section_title(self.t("确认安装状态", "Verify installation")))
                .child(banner(error.clone(), true))
                .child(div().text_sm().child(self.t("完成或关闭安装器后重试。组件版本不一致时，请重新运行完整安装包。", "Finish or close the installer, then retry. If component versions differ, run the full installer again.")))
                .child(button_row()
                    .child(Button::new("verify-update").primary().label(self.t("验证新版本", "Verify new version")).disabled(!idle).on_click(cx.listener(|s, _, _, cx| { s.updates.recover(false); cx.notify(); })))
                    .child(Button::new("resume-old-version").label(self.t("已取消安装，恢复当前版本", "Installation cancelled; resume current version")).disabled(!idle).on_click(cx.listener(|s, _, _, cx| { s.updates.recover(true); cx.notify(); })))));
        }
        if let Some(error) = &self.updates.error {
            page = page.child(banner(error.clone(), true));
        }
        if let Some(message) = &self.updates.message {
            page = page.child(banner(message.clone(), false));
        }
        if self.updates.source == InstallSource::Unknown && self.updates.target.is_some() {
            page = page.child(
                card(compact)
                    .child(section_title(
                        self.t("确认安装来源", "Choose installation source"),
                    ))
                    .child(div().text_sm().child(self.t(
                        "无法确认 Homebrew 记录，请选择这份应用的安装方式。",
                        "Homebrew inventory could not be read. Select how this app was installed.",
                    )))
                    .child(
                        button_row()
                            .child(
                                Button::new("source-pkg")
                                    .label(self.t("直接安装包", "Direct installer"))
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.set_update_source(InstallSource::Package, cx)
                                    })),
                            )
                            .child(Button::new("source-brew").label("Homebrew").on_click(
                                cx.listener(|s, _, _, cx| {
                                    s.set_update_source(InstallSource::Homebrew, cx)
                                }),
                            )),
                    ),
            );
        }
        if let Some(offer) = &self.updates.offer {
            let url = offer.release_url.clone();
            let mut release = card(compact)
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!(
                            "{} {}",
                            self.t("新版本", "New version"),
                            offer.version
                        )),
                )
                .child(
                    Button::new("update-release-notes")
                        .ghost()
                        .label(self.t("查看发行说明", "View release notes"))
                        .on_click(move |_, _, _| {
                            let url = url.clone();
                            std::thread::spawn(move || {
                                let _ = open::that(url);
                            });
                        }),
                );
            if self.updates.source == InstallSource::Homebrew {
                release = release.child(div().text_sm().child(self.t("这份应用由 Homebrew 管理。退出后使用下方命令升级。", "This installation is managed by Homebrew. Quit, then use this command to upgrade.")))
                    .child(div().text_xs().child(updater::BREW_COMMAND))
                    .child(Button::new("copy-brew").label(self.t("复制命令", "Copy command")).on_click(|_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(updater::BREW_COMMAND.into()))));
            } else if let Some(file) = &self.updates.verified {
                let folder = file.path.parent().unwrap().to_path_buf();
                release = release
                    .child(div().text_sm().child(format!(
                        "{}: {}",
                        self.t("已校验", "Verified"),
                        file.offer.installer.name
                    )))
                    .child(
                        Button::new("show-update-download")
                            .ghost()
                            .label(self.t("显示安装包", "Show installer"))
                            .on_click(move |_, _, _| {
                                let folder = folder.clone();
                                std::thread::spawn(move || {
                                    let _ = open::that(folder);
                                });
                            }),
                    );
                if self.updates.manual_install {
                    let command = updater::platform::apt_command(&file.path);
                    release = release.child(div().text_sm().child(self.t("未找到 DEB 图形安装器。退出后在终端执行下方命令。", "No DEB installer was found. Quit, then run this command in your terminal.")))
                        .child(div().text_xs().child(command.clone()))
                        .child(Button::new("copy-apt").label(self.t("复制命令", "Copy command")).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(command.clone()))));
                }
            } else if self.updates.phase == Phase::Downloading {
                let (done, total) = self.updates.progress;
                let ratio = if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                };
                release = release
                    .child(div().text_sm().child(format!(
                        "{:.1} / {:.1} MB · {:.0}%",
                        done as f64 / 1048576.,
                        total as f64 / 1048576.,
                        ratio * 100.
                    )))
                    .child(
                        div()
                            .w_full()
                            .h(px(6.))
                            .rounded_full()
                            .bg(rgb(BORDER))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(ratio))
                                    .bg(rgb(ACCENT))
                                    .rounded_full(),
                            ),
                    )
                    .child(
                        Button::new("cancel-update-download")
                            .label(self.t("取消下载", "Cancel download"))
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.updates.cancel.store(true, Ordering::Release);
                                cx.notify();
                            })),
                    );
            } else {
                release = release.child(
                    Button::new("download-update")
                        .primary()
                        .label(self.t("下载更新", "Download update"))
                        .disabled(!idle || self.updates.source != InstallSource::Package)
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.updates.message = None;
                            s.updates.download();
                            cx.notify();
                        })),
                );
            }
            if self.updates.verified.is_some() || self.updates.source == InstallSource::Homebrew {
                release = release.child(
                    Button::new("install-update")
                        .primary()
                        .label(
                            if self.updates.source == InstallSource::Homebrew
                                || self.updates.manual_install
                            {
                                self.t("退出并准备升级", "Quit to upgrade")
                            } else {
                                self.t("退出并安装", "Quit and install")
                            },
                        )
                        .disabled(
                            !idle
                                || self.modal
                                || self.activity.is_some()
                                || !self.connected()
                                || self.updates.recovery.is_some(),
                        )
                        .on_click(cx.listener(|s, _, w, cx| s.confirm_update(w, cx))),
                );
            }
            release = release.child(div().text_xs().text_color(rgb(MUTED)).child(self.t("完整安装包经 SHA256 校验。当前试用包未签名；安装完成后重新打开应用即可恢复标签页。", "Full installers are checked with SHA256. Trial packages are unsigned. Reopen the app after installation to restore your tabs.")));
            page = page.child(release);
        }
        page
    }
}
