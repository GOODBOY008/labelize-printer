use crate::{
    i18n::Language,
    studio_worker::{self, Preview},
    ui::{ACCENT, BORDER, MUTED},
};
use anyhow::{ensure, Context as _, Result};
use gpui::{prelude::*, *};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    Disableable, Sizable,
};
use labelize_printer_core::{
    design::{DesignDocument, DesignObject, ObjectKind},
    document::{self, DocumentKind},
    paths::atomic_write,
    AppPaths, OutputFormat, Settings,
};
use std::{path::PathBuf, sync::Arc};

pub struct Studio {
    pub language: Language,
    paths: AppPaths,
    options: Settings,
    configured: bool,
    source: Vec<u8>,
    kind: DocumentKind,
    design: Option<DesignDocument>,
    file: Option<PathBuf>,
    selected: Option<usize>,
    fields: Vec<Entity<InputState>>,
    editor: Entity<InputState>,
    editable: bool,
    dirty: bool,
    stale: bool,
    revision: u64,
    busy: bool,
    close_prompt: bool,
    notice: Option<Result<String, String>>,
    preview: Option<Preview>,
    page: usize,
    image: Option<Arc<Image>>,
    outputs: Vec<PathBuf>,
    undo: Vec<DesignDocument>,
    redo: Vec<DesignDocument>,
    drag: Option<(Point<Pixels>, f64, f64)>,
}

impl Studio {
    pub fn new(
        paths: AppPaths,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fields: Vec<_> = (0..9)
            .map(|_| cx.new(|cx| InputState::new(window, cx)))
            .collect();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(14)
                .soft_wrap(false)
        });
        for input in fields.iter().chain(std::iter::once(&editor)) {
            cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) && this.form_changed(cx) {
                    this.changed(cx);
                }
            })
            .detach();
        }
        let mut this = Self {
            language,
            paths,
            options: Settings::default(),
            configured: false,
            source: b"^XA^PW816^LL1216^FO40,40^A0N,40,40^FDLabelize Printer^FS^XZ".to_vec(),
            kind: DocumentKind::Zpl,
            design: None,
            file: None,
            selected: None,
            fields,
            editor,
            editable: true,
            dirty: false,
            stale: true,
            revision: 0,
            busy: false,
            close_prompt: false,
            notice: None,
            preview: None,
            page: 0,
            image: None,
            outputs: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            drag: None,
        };
        this.sync_form(window, cx);
        this
    }
    pub fn configure(&mut self, settings: &Settings, window: &mut Window, cx: &mut Context<Self>) {
        if !self.configured {
            self.options.output_dir = settings.output_dir.clone();
            self.options.dpmm = settings.dpmm;
            self.options.raw_port = settings.raw_port;
            self.options.ipp_port = settings.ipp_port;
            if !self.dirty {
                self.options.width_mm = settings.width_mm;
                self.options.height_mm = settings.height_mm;
                self.sync_form(window, cx);
            }
            self.configured = true;
        }
    }
    fn t(&self, zh: &'static str, en: &'static str) -> &'static str {
        match self.language {
            Language::Chinese => zh,
            Language::English => en,
        }
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.stale = true;
        self.revision += 1;
        self.outputs.clear();
        cx.notify();
    }
    fn values(&self) -> Vec<String> {
        let o = self
            .design
            .as_ref()
            .and_then(|d| self.selected.and_then(|i| d.objects.get(i)));
        let (w, h) = self
            .design
            .as_ref()
            .map(|d| (d.width_mm, d.height_mm))
            .unwrap_or((self.options.width_mm, self.options.height_mm));
        vec![
            number(w),
            number(h),
            number(o.map_or(0., |o| o.x)),
            number(o.map_or(0., |o| o.y)),
            number(o.map_or(50., |o| o.width)),
            number(o.map_or(25., |o| o.height)),
            number(o.map_or(4., |o| o.font_mm)),
            number(o.map_or(0.5, |o| o.stroke_mm)),
            o.map_or(String::new(), |o| o.text.clone()),
        ]
    }
    fn form_changed(&self, cx: &App) -> bool {
        self.fields
            .iter()
            .zip(self.values())
            .any(|(f, v)| f.read(cx).value().as_str() != v)
            || (self.design.is_none()
                && self.editable
                && self.editor.read(cx).value().as_bytes() != self.source)
    }
    fn sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (field, value) in self.fields.iter().zip(self.values()) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        let source = if self.editable {
            String::from_utf8(self.source.clone()).unwrap_or_default()
        } else {
            String::new()
        };
        self.editor
            .update(cx, |input, cx| input.set_value(source, window, cx));
        cx.notify();
    }
    fn remember(&mut self) {
        if let Some(design) = &self.design {
            self.undo.push(design.clone());
        }
        // Font/image payloads are bounded together across the undo stack.
        while self.undo.len() > 20
            || self
                .undo
                .iter()
                .map(|d| {
                    d.font.len()
                        + d.objects
                            .iter()
                            .map(|o| o.image.len() + o.text.len())
                            .sum::<usize>()
                })
                .sum::<usize>()
                > 64 * 1024 * 1024
        {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn apply_fields(&mut self, cx: &mut Context<Self>) -> Result<()> {
        let n = |i: usize| -> Result<f64> {
            self.fields[i]
                .read(cx)
                .value()
                .parse::<f64>()
                .context("Enter a valid number in millimetres")
        };
        let (w, h) = (n(0)?, n(1)?);
        let mut settings = self.options.clone();
        settings.width_mm = w;
        settings.height_mm = h;
        settings.validate()?;
        if let Some(design) = &self.design {
            let mut next = design.clone();
            next.width_mm = w;
            next.height_mm = h;
            if let Some(o) = self.selected.and_then(|i| next.objects.get_mut(i)) {
                o.x = n(2)?;
                o.y = n(3)?;
                o.width = n(4)?;
                o.height = n(5)?;
                o.font_mm = n(6)?;
                o.stroke_mm = n(7)?;
                o.text = self.fields[8].read(cx).value().to_string();
            }
            next.validate(settings.dpmm)?;
            if &next != design {
                self.remember();
                self.design = Some(next);
                self.changed(cx);
            }
        } else if self.editable {
            self.source = self.editor.read(cx).value().as_bytes().to_vec();
        }
        self.options = settings;
        Ok(())
    }
    fn payload(&mut self, cx: &mut Context<Self>) -> Result<Vec<u8>> {
        self.apply_fields(cx)?;
        let bytes = if let Some(d) = &self.design {
            serde_json::to_vec_pretty(d)?
        } else {
            self.source.clone()
        };
        ensure!(
            !bytes.is_empty() && bytes.len() as u64 <= labelize_printer_core::MAX_INPUT_BYTES,
            "Input must be 1 byte to 64 MiB"
        );
        Ok(bytes)
    }
    fn fail(&mut self, error: impl std::fmt::Display, cx: &mut Context<Self>) {
        self.notice = Some(Err(error.to_string()));
        cx.notify();
    }
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.dirty && !self.form_changed(cx) {
            return true;
        }
        if self.close_prompt {
            return false;
        }
        self.close_prompt = true;
        let prompt = window.prompt(
            PromptLevel::Warning,
            self.t("放弃未保存的修改？", "Discard unsaved changes?"),
            Some(self.t(
                "已提交的输出任务会在后台继续。取消可返回保存标签文件。",
                "Submitted output jobs continue in the background. Cancel to save the label file.",
            )),
            &[
                PromptButton::cancel(self.t("取消", "Cancel")),
                PromptButton::new(self.t("放弃并退出", "Discard and quit")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.close_prompt = false;
                if result == Ok(1) {
                    cx.quit();
                }
                cx.notify();
            });
        })
        .detach();
        false
    }
    fn confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        next: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.busy {
            return;
        }
        if !self.dirty && !self.form_changed(cx) {
            next(self, window, cx);
            return;
        }
        self.busy = true;
        let prompt = window.prompt(
            PromptLevel::Warning,
            self.t("放弃未保存的修改？", "Discard unsaved changes?"),
            Some(self.t(
                "可先取消并保存标签文件或设计模板。",
                "Cancel to save the label file or design template first.",
            )),
            &[
                PromptButton::cancel(self.t("取消", "Cancel")),
                PromptButton::new(self.t("放弃修改", "Discard changes")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if answer == Ok(1) {
                    next(this, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx, |this, window, cx| this.pick_file(0, window, cx));
    }
    fn pick_file(&mut self, purpose: u8, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(self.t("打开文件", "Open file").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let choice = choice.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match choice {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => {
                        this.load(paths[0].clone(), purpose, window, cx)
                    }
                    Ok(Err(e)) => this.fail(e, cx),
                    _ => cx.notify(),
                }
            });
        })
        .detach();
    }
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx, move |this, window, cx| {
            this.load(path, 0, window, cx)
        });
    }
    fn load(&mut self, path: PathBuf, purpose: u8, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let read = cx.background_executor().spawn(async move {
            let bytes = document::read_input(&path)?;
            Ok::<_, anyhow::Error>((path, bytes))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                let result = (|| -> Result<()> {
                    let (path, bytes) = result?;
                    if purpose != 0 {
                        this.apply_fields(cx)?;
                        if purpose == 1 {
                            let image = labelize_printer_core::design::normalize_image(&bytes)?;
                            this.remember();
                            if let Some(d) = &mut this.design {
                                ensure!(d.objects.len() < 100, "At most 100 objects");
                                let mut o = DesignObject::new(ObjectKind::Image);
                                o.image = image;
                                fit(&mut o, d);
                                d.objects.push(o);
                                this.selected = Some(d.objects.len() - 1);
                            }
                        } else {
                            ensure!(bytes.len() <= 32 * 1024 * 1024, "Font exceeds 32 MiB");
                            this.remember();
                            if let Some(d) = &mut this.design {
                                d.font = bytes;
                            }
                        }
                        this.changed(cx);
                    } else {
                        let kind = DocumentKind::detect(&path, &bytes);
                        let design = if kind == DocumentKind::Design {
                            let d: DesignDocument = serde_json::from_slice(&bytes)?;
                            d.validate(this.options.dpmm)?;
                            Some(d)
                        } else {
                            None
                        };
                        this.design = design;
                        this.kind = kind;
                        this.source = if this.design.is_some() {
                            Vec::new()
                        } else {
                            bytes
                        };
                        this.file = Some(path);
                        this.selected = this
                            .design
                            .as_ref()
                            .and_then(|d| (!d.objects.is_empty()).then_some(0));
                        this.editable = this.design.is_none()
                            && this.source.len() <= 1024 * 1024
                            && std::str::from_utf8(&this.source).is_ok_and(|s| {
                                s.chars()
                                    .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
                            });
                        this.dirty = false;
                        this.stale = true;
                        this.revision += 1;
                        this.undo.clear();
                        this.redo.clear();
                        this.preview = None;
                        this.image = None;
                        this.outputs.clear();
                    }
                    this.sync_form(window, cx);
                    this.render_preview(window, cx);
                    Ok(())
                })();
                if let Err(e) = result {
                    this.fail(e, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn new_design(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx, |this, window, cx| {
            this.design = Some(DesignDocument::default());
            this.kind = DocumentKind::Design;
            this.selected = Some(0);
            this.file = None;
            this.source.clear();
            this.editable = false;
            this.undo.clear();
            this.redo.clear();
            this.preview = None;
            this.image = None;
            this.changed(cx);
            this.sync_form(window, cx);
            this.render_preview(window, cx);
        });
    }
    fn render_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let bytes = match self.payload(cx) {
            Ok(b) => b,
            Err(e) => {
                self.fail(e, cx);
                return;
            }
        };
        self.busy = true;
        self.notice = None;
        let revision = self.revision;
        let paths = self.paths.clone();
        let kind = self.kind;
        let settings = self.options.clone();
        let work = cx
            .background_executor()
            .spawn(async move { studio_worker::preview(&paths, bytes, kind, settings) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                if revision == this.revision {
                    match result {
                        Ok(preview) => {
                            this.preview = Some(preview);
                            this.page = 0;
                            this.stale = false;
                            this.show_page(cx);
                        }
                        Err(e) => this.fail(e, cx),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn show_page(&mut self, cx: &mut Context<Self>) {
        self.image = self
            .preview
            .as_ref()
            .and_then(|p| {
                p.pages
                    .get(self.page)
                    .and_then(|page| std::fs::read(p.directory.path().join(&page.file)).ok())
            })
            .map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)));
        cx.notify();
    }
    fn export(&mut self, format: OutputFormat, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let bytes = match self.payload(cx) {
            Ok(b) => b,
            Err(e) => {
                self.fail(e, cx);
                return;
            }
        };
        let mut settings = self.options.clone();
        settings.format = format;
        let paths = self.paths.clone();
        let kind = self.kind;
        let title = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Label design".into());
        self.busy = true;
        self.notice = None;
        let work = cx
            .background_executor()
            .spawn(async move { studio_worker::export(&paths, bytes, kind, settings, title) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(files) => {
                        this.outputs = files;
                        this.notice = Some(Ok(this
                            .t(
                                "已保存。可打开输出文件，或到最近任务查看。",
                                "Saved. Open the output or view it in Recent jobs.",
                            )
                            .into()));
                    }
                    Err(e) => this.fail(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let bytes = match self.payload(cx) {
            Ok(b) => b,
            Err(e) => {
                self.fail(e, cx);
                return;
            }
        };
        let name = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                if self.design.is_some() {
                    "label.labelize.json".into()
                } else {
                    format!("label.{}", self.kind.name().to_ascii_lowercase())
                }
            });
        let choice = cx.prompt_for_new_path(&self.options.output_dir, Some(&name));
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let selected = choice.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match selected {
                    Ok(Ok(Some(path))) => match atomic_write(&path, &bytes) {
                        Ok(()) => {
                            this.file = Some(path);
                            this.dirty = false;
                            this.notice =
                                Some(Ok(this.t("标签文件已保存。", "Label file saved.").into()));
                        }
                        Err(e) => this.fail(e, cx),
                    },
                    Ok(Err(e)) => this.fail(e, cx),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.t("输出目录", "Output folder").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = choice.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match selected {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => {
                        this.options.output_dir = paths[0].clone()
                    }
                    Ok(Err(e)) => this.fail(e, cx),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn add(&mut self, kind: ObjectKind, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.apply_fields(cx) {
            self.fail(e, cx);
            return;
        }
        self.remember();
        if let Some(d) = &mut self.design {
            if d.objects.len() >= 100 {
                self.fail("At most 100 objects", cx);
                return;
            }
            let mut object = DesignObject::new(kind);
            fit(&mut object, d);
            d.objects.push(object);
            self.selected = Some(d.objects.len() - 1);
        }
        self.changed(cx);
        self.sync_form(window, cx);
        self.render_preview(window, cx);
    }
    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.apply_fields(cx) {
            self.fail(e, cx);
            return;
        }
        self.selected = Some(index);
        self.sync_form(window, cx);
    }
    fn history(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        let next = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(next) = next {
            if let Some(old) = self.design.replace(next) {
                if redo {
                    self.undo.push(old);
                } else {
                    self.redo.push(old);
                }
            }
            self.selected = self
                .design
                .as_ref()
                .and_then(|d| (!d.objects.is_empty()).then_some(0));
            self.changed(cx);
            self.sync_form(window, cx);
            self.render_preview(window, cx);
        }
    }
    fn object_name(&self, kind: ObjectKind) -> &'static str {
        match kind {
            ObjectKind::Text => self.t("文字", "Text"),
            ObjectKind::Code128 => "Code 128",
            ObjectKind::Qr => self.t("二维码", "QR code"),
            ObjectKind::Rectangle => self.t("矩形", "Rectangle"),
            ObjectKind::Image => self.t("图片", "Image"),
        }
    }
    fn field(&self, index: usize, label: &'static str) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .min_w_0()
            .child(
                div()
                    .w_full()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(label),
            )
            .child(Input::new(&self.fields[index]).disabled(self.busy))
    }
    fn finish_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.sync_form(window, cx);
            self.render_preview(window, cx);
        }
    }
}

fn number(value: f64) -> String {
    let s = format!("{value:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
fn fit(o: &mut DesignObject, d: &DesignDocument) {
    o.x = o.x.min(d.width_mm / 4.);
    o.y = o.y.min(d.height_mm / 4.);
    o.width = o.width.min(d.width_mm - o.x);
    o.height = o.height.min(d.height_mm - o.y);
    o.stroke_mm = o.stroke_mm.min(o.width.min(o.height));
}
fn row() -> Div {
    div().flex().flex_wrap().gap_2().min_w_0()
}
fn field_row() -> Div {
    div().grid().grid_cols(2).gap_2().w_full().min_w_0()
}
fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap_3()
        .p_4()
        .bg(rgb(0xffffff))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded_lg()
}

impl Render for Studio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = window.viewport_size().width < px(850.);
        let title = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.t("未命名标签", "Untitled label").into());
        let toolbar = row()
            .child(
                Button::new("open-label")
                    .label(self.t("打开文件…", "Open file…"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|s, _, w, cx| s.open(w, cx))),
            )
            .child(
                Button::new("new-design")
                    .label(self.t("新建设计", "New design"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|s, _, w, cx| s.new_design(w, cx))),
            )
            .child(
                Button::new("save-label")
                    .label(self.t("另存标签…", "Save label as…"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|s, _, w, cx| s.save(w, cx))),
            )
            .child(
                Button::new("render-label")
                    .label(self.t("更新预览", "Update preview"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|s, _, w, cx| s.render_preview(w, cx))),
            )
            .child(
                Button::new("export-pdf")
                    .primary()
                    .label(self.t("保存 PDF", "Save PDF"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|s, _, w, cx| s.export(OutputFormat::Pdf, w, cx))),
            )
            .child(
                Button::new("export-png")
                    .label(self.t("保存 PNG", "Save PNG"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|s, _, w, cx| s.export(OutputFormat::Png, w, cx))),
            );
        let mut controls = panel()
            .when(!narrow, |d| d.w(px(320.)).flex_shrink_0())
            .when(narrow, |d| d.w_full())
            .child(
                field_row()
                    .child(self.field(0, self.t("标签宽度 mm", "Label width mm")))
                    .child(self.field(1, self.t("标签高度 mm", "Label height mm"))),
            )
            .child(row().children([8, 12].into_iter().map(|dpmm| {
                Button::new(("studio-dpmm", dpmm as usize))
                    .small()
                    .label(format!(
                        "{dpmm} dpmm{}",
                        if self.options.dpmm == dpmm {
                            " ✓"
                        } else {
                            ""
                        }
                    ))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.options.dpmm = dpmm;
                        s.changed(cx);
                    }))
            })));
        if let Some(design) = &self.design {
            let mut add = row();
            for kind in [
                ObjectKind::Text,
                ObjectKind::Code128,
                ObjectKind::Qr,
                ObjectKind::Rectangle,
            ] {
                add = add.child(
                    Button::new(("add-object", kind as usize))
                        .small()
                        .label(format!("+ {}", self.object_name(kind)))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |s, _, w, cx| s.add(kind, w, cx))),
                );
            }
            controls = controls
                .child(
                    add.child(
                        Button::new("add-image")
                            .small()
                            .label(self.t("+ 图片…", "+ Image…"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|s, _, w, cx| s.pick_file(1, w, cx))),
                    ),
                )
                .child(
                    row()
                        .child(
                            Button::new("undo-design")
                                .small()
                                .label(self.t("撤销", "Undo"))
                                .disabled(self.busy || self.undo.is_empty())
                                .on_click(cx.listener(|s, _, w, cx| s.history(false, w, cx))),
                        )
                        .child(
                            Button::new("redo-design")
                                .small()
                                .label(self.t("重做", "Redo"))
                                .disabled(self.busy || self.redo.is_empty())
                                .on_click(cx.listener(|s, _, w, cx| s.history(true, w, cx))),
                        )
                        .child(
                            Button::new("font-design")
                                .small()
                                .label(self.t("导入字体…", "Import font…"))
                                .disabled(self.busy)
                                .on_click(cx.listener(|s, _, w, cx| s.pick_file(2, w, cx))),
                        ),
                );
            let mut layers = row();
            for (i, o) in design.objects.iter().enumerate() {
                layers = layers.child(
                    Button::new(("object", i))
                        .small()
                        .label(format!(
                            "{} {}{}",
                            i + 1,
                            self.object_name(o.kind),
                            if self.selected == Some(i) { " ✓" } else { "" }
                        ))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |s, _, w, cx| s.select(i, w, cx))),
                );
            }
            controls = controls
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.t(
                            "对象 · 拖动画布上的对象可移动",
                            "Objects · drag on canvas to move",
                        )),
                )
                .child(layers);
            if let Some(o) = self.selected.and_then(|i| design.objects.get(i)) {
                controls = controls
                    .child(
                        field_row()
                            .child(self.field(2, "X mm"))
                            .child(self.field(3, "Y mm")),
                    )
                    .child(
                        field_row()
                            .child(self.field(4, self.t("宽度 mm", "Width mm")))
                            .child(self.field(5, self.t("高度 mm", "Height mm"))),
                    );
                if matches!(
                    o.kind,
                    ObjectKind::Text | ObjectKind::Code128 | ObjectKind::Qr
                ) {
                    controls = controls.child(self.field(8, self.t("内容", "Content")));
                }
                if o.kind == ObjectKind::Text {
                    controls = controls.child(self.field(6, self.t("字号 mm", "Font size mm")));
                }
                if o.kind == ObjectKind::Rectangle {
                    controls = controls.child(self.field(7, self.t("边框 mm", "Border mm")));
                }
                controls = controls.child(
                    Button::new("delete-object")
                        .label(self.t("删除对象", "Delete object"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|s, _, w, cx| {
                            s.remember();
                            if let (Some(i), Some(d)) = (s.selected, &mut s.design) {
                                d.objects.remove(i);
                                s.selected = (!d.objects.is_empty())
                                    .then_some(i.min(d.objects.len().saturating_sub(1)));
                            }
                            s.changed(cx);
                            s.sync_form(w, cx);
                            s.render_preview(w, cx);
                        })),
                );
            }
            controls=controls.child(div().text_xs().text_color(rgb(MUTED)).child(self.t("模板内嵌图片和导入的字体。中文文字请导入支持中文的字体。","Templates embed images and imported fonts. For CJK text, import a font with the required glyphs.")));
        } else {
            let mut formats = row();
            for (i, kind) in DocumentKind::LANGUAGES.into_iter().enumerate() {
                formats = formats.child(
                    Button::new(("file-language", i))
                        .small()
                        .label(format!(
                            "{}{}",
                            kind.name(),
                            if self.kind == kind { " ✓" } else { "" }
                        ))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |s, _, _, cx| {
                            s.kind = kind;
                            s.changed(cx);
                        })),
                );
            }
            controls = controls
                .child(formats)
                .child(div().text_xs().text_color(rgb(MUTED)).child(self.t(
                    "文件声明的尺寸优先于上述缺省尺寸。",
                    "Dimensions declared in the file override these defaults.",
                )));
            controls = if self.editable {
                controls.child(
                    Input::new(&self.editor)
                        .h(px(280.))
                        .tab_index(100)
                        .disabled(self.busy),
                )
            } else {
                controls.child(div().text_sm().child(self.t("此文件含二进制数据、控制字节或超过 1 MiB。按原始字节预览和输出，源码编辑已关闭。","This file contains binary/control bytes or exceeds 1 MiB. Preview and output preserve its original bytes; source editing is disabled.")))
            };
        }
        let status = if self.busy {
            self.t("正在处理…", "Working…")
        } else if self.stale {
            self.t(
                "预览待更新；输出使用当前内容。",
                "Preview needs updating; output uses current content.",
            )
        } else {
            self.t("预览", "Preview")
        };
        let mut preview_panel = panel()
            .flex_1()
            .items_center()
            .when(narrow, |p| p.w_full())
            .child(
                div()
                    .w_full()
                    .h(px(42.))
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(status),
            );
        if let (Some(image), Some(preview)) = (&self.image, &self.preview) {
            if let Some(page) = preview.pages.get(self.page) {
                let available = if narrow {
                    f32::from(window.viewport_size().width) - 112.
                } else {
                    (f32::from(window.viewport_size().width) - 480.).min(600.)
                };
                let scale =
                    (available.max(150.) / page.width_mm as f32).min(470. / page.height_mm as f32);
                let mut canvas = div()
                    .id("design-canvas")
                    .relative()
                    .w(px(page.width_mm as f32 * scale))
                    .h(px(page.height_mm as f32 * scale))
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(img(image.clone()).size_full());
                if let Some(design) = &self.design {
                    for (i, o) in design.objects.iter().enumerate() {
                        canvas = canvas.child(
                            div()
                                .id(("canvas-object", i))
                                .absolute()
                                .left(px(o.x as f32 * scale))
                                .top(px(o.y as f32 * scale))
                                .w(px(o.width as f32 * scale))
                                .h(px(o.height as f32 * scale))
                                .cursor_move()
                                .border_1()
                                .border_color(if self.selected == Some(i) {
                                    rgb(ACCENT)
                                } else {
                                    rgba(0x315efb33)
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |s, e: &MouseDownEvent, w, cx| {
                                        if s.busy {
                                            return;
                                        }
                                        if let Err(error) = s.apply_fields(cx) {
                                            s.fail(error, cx);
                                            return;
                                        }
                                        s.select(i, w, cx);
                                        if s.selected != Some(i) {
                                            return;
                                        }
                                        s.remember();
                                        if let Some(o) =
                                            s.design.as_ref().and_then(|d| d.objects.get(i))
                                        {
                                            s.drag = Some((e.position, o.x, o.y));
                                        }
                                        cx.stop_propagation();
                                    }),
                                ),
                        );
                    }
                }
                preview_panel = preview_panel
                    .child(
                        canvas
                            .on_mouse_move(cx.listener(move |s, e: &MouseMoveEvent, w, cx| {
                                if let Some((start, x, y)) = s.drag {
                                    if e.pressed_button != Some(MouseButton::Left) {
                                        s.finish_drag(w, cx);
                                        return;
                                    }
                                    if let (Some(i), Some(d)) = (s.selected, &mut s.design) {
                                        let o = &mut d.objects[i];
                                        o.x = (x + f64::from(
                                            f32::from(e.position.x - start.x) / scale,
                                        ))
                                        .clamp(0., d.width_mm - o.width);
                                        o.y = (y + f64::from(
                                            f32::from(e.position.y - start.y) / scale,
                                        ))
                                        .clamp(0., d.height_mm - o.height);
                                        s.changed(cx);
                                    }
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|s, _, w, cx| s.finish_drag(w, cx)),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|s, _, w, cx| s.finish_drag(w, cx)),
                            ),
                    )
                    .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                        "{} × {} mm · {} × {} px",
                        number(page.width_mm),
                        number(page.height_mm),
                        page.width,
                        page.height
                    )))
                    .child(
                        row()
                            .child(
                                Button::new("previous-label")
                                    .label("‹")
                                    .disabled(self.busy || self.page == 0)
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.page -= 1;
                                        s.show_page(cx);
                                    })),
                            )
                            .child(div().py_2().child(format!(
                                "{} / {}",
                                self.page + 1,
                                preview.pages.len()
                            )))
                            .child(
                                Button::new("next-label")
                                    .label("›")
                                    .disabled(self.busy || self.page + 1 >= preview.pages.len())
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.page += 1;
                                        s.show_page(cx);
                                    })),
                            ),
                    );
            }
        } else {
            preview_panel = preview_panel.child(div().py_8().text_color(rgb(MUTED)).child(self.t(
                "打开标签文件，或新建可视化设计。",
                "Open a label file or create a visual design.",
            )));
        }
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .min_w_0()
            .child(toolbar)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "{}{} · {}",
                        title,
                        if self.dirty { " *" } else { "" },
                        self.kind.name()
                    )),
            );
        if let Some(message) = &self.notice {
            body = body.child(
                div()
                    .p_3()
                    .rounded_md()
                    .text_sm()
                    .bg(rgb(if message.is_err() { 0xfbeaec } else { 0xe7f5ed }))
                    .child(match message {
                        Ok(s) | Err(s) => s.clone(),
                    }),
            );
        }
        body = body
            .child(
                div()
                    .flex()
                    .gap_4()
                    .min_w_0()
                    .items_start()
                    .when(narrow, |d| d.flex_col())
                    .child(controls)
                    .child(preview_panel),
            )
            .child(
                row()
                    .child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                        "{}: {}",
                        self.t("输出目录", "Output folder"),
                        self.options.output_dir.display()
                    )))
                    .child(
                        Button::new("studio-folder")
                            .small()
                            .label(self.t("更改…", "Change…"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|s, _, w, cx| s.folder(w, cx))),
                    ),
            );
        if let Some(file) = self.outputs.first() {
            let file = file.clone();
            body = body.child(
                Button::new("open-studio-output")
                    .label(self.t("打开输出文件", "Open output file"))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        if let Err(e) = open::that_detached(&file) {
                            s.fail(e, cx);
                        }
                    })),
            );
        }
        body.on_drop(cx.listener(|s, paths: &ExternalPaths, w, cx| {
            if let Some(path) = paths.paths().first() {
                s.open_path(path.clone(), w, cx);
            }
        }))
    }
}
