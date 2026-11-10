use crate::{
    i18n::Language,
    studio_worker::{self, Preview},
    ui::{ScrollPageDown, ScrollPageUp, ACCENT, BORDER, MUTED},
};
use anyhow::{ensure, Context as _, Result};
use gpui::{prelude::*, *};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    Disableable, IconName, Selectable, Sizable,
};
use labelize_printer::{
    preview_images::{self, PreviewCache, PreviewImages, PreviewKey},
    preview_queue::{PreviewQueue, PreviewState},
    session::{self, Draft},
};
use labelize_printer_core::{
    design::{
        Alignment, Background, DesignDocument, DesignObject, HorizontalAlign, ImageFit, LayerOrder,
        LineDirection, MatrixShape, ObjectKind, QrEcc, TextFont, VerticalAlign, TEMPLATE_VERSION,
    },
    document::{self, DocumentKind},
    paths::atomic_write,
    AppPaths, OutputFormat, RenderMode, Settings,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[path = "studio_properties.rs"]
mod properties;

#[derive(Clone, Copy)]
struct ObjectDrag {
    origin: Point<Pixels>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    resize: bool,
}

/// One document session. All editor state and undo history belongs to this entity.
pub struct Studio {
    pub id: String,
    pub language: Language,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    pub persist_version: u64,
    pub close_ready: bool,
    fingerprint: Option<String>,
    paths: AppPaths,
    options: Settings,
    source: Vec<u8>,
    kind: DocumentKind,
    design: Option<DesignDocument>,
    selected: Option<usize>,
    fields: Vec<Entity<InputState>>,
    editor: Entity<InputState>,
    editable: bool,
    stale: bool,
    revision: u64,
    loading: bool,
    saving: bool,
    submitting: bool,
    rendering: PreviewState,
    close_after_save: bool,
    close_prompt: bool,
    notice: Option<Result<String, String>>,
    preview_error: Option<String>,
    preview: Option<Preview>,
    page: usize,
    display: Option<Arc<PreviewImages>>,
    images: Arc<Mutex<PreviewCache>>,
    image_loading: bool,
    image_request: u64,
    image_cancel: Arc<AtomicBool>,
    outputs: Vec<PathBuf>,
    undo: Vec<DesignDocument>,
    redo: Vec<DesignDocument>,
    drag: Option<ObjectDrag>,
    queue: Arc<PreviewQueue<Preview>>,
    due: Option<Instant>,
    active: bool,
    zoom: Option<f32>,
    source_mode: bool,
    inspector_open: bool,
    layers_open: bool,
    canvas_scroll: ScrollHandle,
    inspector_scroll: ScrollHandle,
    layers_scroll: ScrollHandle,
    canvas_size: Size<Pixels>,
    window_handle: AnyWindowHandle,
    focus: FocusHandle,
}

impl Studio {
    pub fn new(
        paths: AppPaths,
        language: Language,
        draft: Draft,
        queue: Arc<PreviewQueue<Preview>>,
        images: Arc<Mutex<PreviewCache>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|this, cx| {
            this.forget_preview();
            this.release_images(cx);
        })
        .detach();
        let fields: Vec<_> = (0..session::DESIGN_FIELD_COUNT)
            .map(|index| {
                cx.new(|cx| {
                    let input = InputState::new(window, cx);
                    if index == 8 {
                        input.multi_line(true).rows(3)
                    } else {
                        input
                    }
                })
            })
            .collect();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(20)
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
        let raw_fields = draft.fields.clone();
        let editable = draft.design.is_none() && editable_source(&draft.source);
        let mut this = Self {
            id: draft.id,
            language,
            file: draft.file,
            dirty: draft.dirty,
            persist_version: 0,
            close_ready: false,
            fingerprint: draft.fingerprint,
            paths,
            options: draft.options,
            source: draft.source,
            kind: draft.kind,
            design: draft.design,
            selected: draft.selected,
            fields,
            editor,
            editable,
            stale: true,
            revision: draft.revision,
            loading: false,
            saving: false,
            submitting: false,
            rendering: PreviewState::default(),
            close_after_save: false,
            close_prompt: false,
            notice: None,
            preview_error: None,
            preview: None,
            page: draft.page,
            display: None,
            images,
            image_loading: false,
            image_request: 0,
            image_cancel: Arc::new(AtomicBool::new(false)),
            outputs: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            drag: None,
            queue,
            due: Some(Instant::now()),
            active: false,
            zoom: draft.zoom,
            source_mode: draft.source_mode,
            inspector_open: false,
            layers_open: false,
            canvas_scroll: ScrollHandle::default(),
            inspector_scroll: ScrollHandle::default(),
            layers_scroll: ScrollHandle::default(),
            canvas_size: Size::default(),
            window_handle: window.window_handle(),
            focus: cx.focus_handle(),
        };
        this.sync_form(window, cx);
        // Restore raw fields after the validated model: even incomplete numbers survive a crash.
        if !raw_fields.is_empty() {
            for (field, value) in this.fields.iter().zip(raw_fields) {
                field.update(cx, |i, cx| i.set_value(value, window, cx));
            }
        }
        this
    }
    pub fn t(&self, zh: &'static str, en: &'static str) -> &'static str {
        match self.language {
            Language::Chinese => zh,
            Language::English => en,
        }
    }
    pub fn title(&self) -> String {
        self.file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.t("未命名设计", "Untitled design").into())
    }
    pub fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.source_mode && self.editable {
            self.editor.update(cx, |e, cx| e.focus(window, cx));
        }
    }
    pub fn format(&self) -> OutputFormat {
        self.options.format
    }
    pub fn is_saving(&self) -> bool {
        self.saving
    }
    pub fn is_submitting(&self) -> bool {
        self.submitting
    }
    pub fn snapshot(&self, cx: &App) -> Draft {
        let mut d = Draft::new(self.options.clone());
        d.id = self.id.clone();
        d.file = self.file.clone();
        d.fingerprint = self.fingerprint.clone();
        d.kind = self.kind;
        d.design = self.design.clone();
        d.source = if self.editable {
            self.editor.read(cx).value().as_bytes().to_vec()
        } else {
            self.source.clone()
        };
        d.fields = if self.drag.is_some() {
            self.values()
        } else {
            self.fields
                .iter()
                .map(|f| f.read(cx).value().to_string())
                .collect()
        };
        d.dirty = self.dirty;
        d.revision = self.revision;
        d.selected = self.selected;
        d.page = self.page;
        d.zoom = self.zoom;
        d.source_mode = self.source_mode;
        d
    }
    pub fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.active == active {
            // Update cancellation can re-arm the visible stale document.
            if active && self.stale && self.due.is_none() {
                self.due = Some(Instant::now());
            }
            return;
        }
        if !active && self.drag.take().is_some() {
            self.sync_form(window, cx);
        }
        self.active = active;
        if active {
            if self.display.is_none() {
                self.show_page(cx);
            }
            if self.stale {
                self.due = Some(Instant::now());
            }
        } else {
            self.queue.cancel(&self.id);
            self.rendering.cancel();
            self.image_request += 1;
            self.image_loading = false;
            self.release_images(cx);
        }
        cx.notify();
    }
    pub fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active
            && !self.loading
            && self.drag.is_none()
            && self.due.is_some_and(|due| Instant::now() >= due)
        {
            self.render_preview(window, cx);
        }
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.render_options_changed(cx);
    }
    // Output preferences live in the session, not in the source file. Changing
    // them invalidates the preview without creating a false unsaved-file prompt.
    fn render_options_changed(&mut self, cx: &mut Context<Self>) {
        self.stale = true;
        self.revision += 1;
        self.persist_version += 1;
        self.outputs.clear();
        self.queue.cancel(&self.id);
        self.rendering.cancel();
        self.due = Some(Instant::now() + Duration::from_millis(500));
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
            o.map_or(String::new(), |o| o.style.name.clone()),
            number(o.map_or(1., |o| o.style.line_spacing)),
            number(o.map_or(0., |o| o.style.corner_radius_mm)),
            number(o.map_or(0., |o| o.style.module_mm)),
            o.map_or(0, |o| o.style.quiet_modules).to_string(),
            o.map_or(0, |o| o.style.pdf417_columns).to_string(),
            o.map_or(2, |o| o.style.pdf417_security).to_string(),
            o.map_or(128, |o| o.style.image_threshold).to_string(),
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
            if let Some(o) = self
                .selected
                .and_then(|i| next.objects.get_mut(i))
                .filter(|o| !o.style.locked)
            {
                o.x = n(2)?;
                o.y = n(3)?;
                o.width = n(4)?;
                o.height = n(5)?;
                o.font_mm = n(6)?;
                o.stroke_mm = n(7)?;
                o.text = self.fields[8].read(cx).value().to_string();
                o.style.name = self.fields[9].read(cx).value().to_string();
                o.style.line_spacing = n(10)?;
                o.style.corner_radius_mm = n(11)?;
                o.style.module_mm = n(12)?;
                o.style.quiet_modules = self.fields[13]
                    .read(cx)
                    .value()
                    .parse()
                    .context("Quiet zone must be a whole number")?;
                o.style.pdf417_columns = self.fields[14]
                    .read(cx)
                    .value()
                    .parse()
                    .context("Columns must be a whole number")?;
                o.style.pdf417_security = self.fields[15]
                    .read(cx)
                    .value()
                    .parse()
                    .context("Error correction must be a whole number")?;
                o.style.image_threshold = self.fields[16]
                    .read(cx)
                    .value()
                    .parse()
                    .context("Image threshold must be 0–255")?;
            }
            next.validate(settings.dpmm)?;
            if &next != design {
                next.version = TEMPLATE_VERSION;
                self.remember();
                self.design = Some(next);
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
            let mut portable = d.clone();
            portable.version = TEMPLATE_VERSION;
            serde_json::to_vec_pretty(&portable)?
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

    pub fn render_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.due = None;
        let bytes = match self.payload(cx) {
            Ok(b) => b,
            Err(e) => {
                self.preview_error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        let revision = self.revision;
        let paths = self.paths.clone();
        let kind = self.kind;
        let settings = self.options.clone();
        let request = self.rendering.begin(revision);
        self.preview_error = None;
        let result = self.queue.submit(self.id.clone(), move |cancel| {
            studio_worker::preview(&paths, bytes, kind, settings, cancel)
                .map_err(|e| format!("{e:#}"))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = result.recv().await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.revision != revision || !this.rendering.finish(request, this.revision) {
                    return;
                }
                match result {
                    Ok(Ok(preview)) => {
                        this.page = this.page.min(preview.pages.len().saturating_sub(1));
                        this.forget_preview();
                        this.preview = Some(preview);
                        this.stale = false;
                        this.preview_error = None;
                        if this.active {
                            this.show_page(cx);
                        }
                    }
                    Ok(Err(e)) if e != "Preview cancelled" => this.preview_error = Some(e),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn release_images(&mut self, cx: &mut App) {
        self.image_cancel.store(true, Ordering::Release);
        let Some(display) = self.display.take() else {
            return;
        };
        // A window may present its previous scene again before drawing a new
        // one. Evicting immediately leaves those sprites pointing at freed GPU
        // textures (MetalAtlas::texture panic). Evict at the next frame boundary
        // and force that same frame to rebuild, including any reactivated image.
        let window = self.window_handle;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| {
                window.on_next_frame(move |window, _| {
                    for image in display.images() {
                        let _ = window.drop_image(image.clone());
                    }
                    window.refresh();
                });
            });
        });
    }
    fn forget_preview(&self) {
        if let Some(preview) = &self.preview {
            self.images
                .lock()
                .unwrap()
                .remove_directory(preview.directory.path());
        }
    }
    fn show_page(&mut self, cx: &mut Context<Self>) {
        self.image_request += 1;
        let ticket = self.image_request;
        self.image_cancel.store(true, Ordering::Release);
        self.image_loading = false;
        let Some(preview) = &self.preview else {
            return;
        };
        let key = PreviewKey {
            directory: preview.directory.path().into(),
            page: self.page,
        };
        let cached = self.images.lock().unwrap().get(&key);
        if let Some(images) = cached {
            // No task, disk IO, PNG decode or placeholder frame on a warm tab switch.
            self.release_images(cx);
            self.display = Some(images);
            log::debug!("preview cache hit: {} page {}", self.id, self.page + 1);
            cx.notify();
            return;
        }
        let directory = key.directory.clone();
        let pages = preview.pages.clone();
        self.image_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.image_cancel.clone();
        self.image_loading = true;
        let page = self.page;
        let work = cx
            .background_executor()
            .spawn(async move { preview_images::load(directory, pages, page, cancel) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |s, cx| {
                if !s.active || s.image_request != ticket {
                    return;
                }
                s.image_loading = false;
                match result {
                    Ok(images) => {
                        s.images.lock().unwrap().insert(key, images.clone());
                        // Keep the last successful frame until the replacement is ready.
                        s.release_images(cx);
                        s.display = Some(images);
                    }
                    Err(e) => {
                        s.stale = true;
                        s.preview_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting || self.loading {
            return;
        }
        let bytes = match self.payload(cx) {
            Ok(b) => b,
            Err(e) => {
                self.fail(e, cx);
                return;
            }
        };
        let paths = self.paths.clone();
        let settings = self.options.clone();
        let kind = self.kind;
        let title = self.title();
        self.submitting = true;
        self.notice = None;
        let work = cx
            .background_executor()
            .spawn(async move { studio_worker::submit(&paths, bytes, kind, settings, title) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let accepted = this.update_in(cx, |this, _, cx| {
                this.submitting = false;
                match result {
                    Ok(job) => {
                        this.notice = Some(Ok(this
                            .t(
                                "已加入后台任务，可以继续编辑。",
                                "Queued in the background. You can keep editing.",
                            )
                            .into()));
                        cx.notify();
                        Some((this.paths.clone(), job.id))
                    }
                    Err(e) => {
                        this.fail(e, cx);
                        None
                    }
                }
            });
            if let Ok(Some((paths, id))) = accepted {
                let result = cx
                    .background_executor()
                    .spawn(async move { studio_worker::wait_for_output(&paths, id) })
                    .await;
                let _ = this.update_in(cx, |this, _, cx| {
                    match result {
                        Ok(files) => {
                            this.outputs = files;
                            this.notice = Some(Ok(this.t("文件已保存。", "Files saved.").into()));
                        }
                        Err(e) => this.fail(e, cx),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_as(false, window, cx);
    }
    pub fn save_as(&mut self, choose: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.loading {
            return;
        }
        let bytes = if self.design.is_some() {
            self.payload(cx)
        } else {
            let b = if self.editable {
                self.editor.read(cx).value().as_bytes().to_vec()
            } else {
                self.source.clone()
            };
            if b.len() as u64 > labelize_printer_core::MAX_INPUT_BYTES {
                Err(anyhow::anyhow!("Input exceeds 64 MiB"))
            } else {
                Ok(b)
            }
        };
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                self.close_after_save = false;
                self.fail(e, cx);
                return;
            }
        };
        let revision = self.revision;
        let known = self.file.clone();
        let expected = self.fingerprint.clone();
        let picker = if choose || known.is_none() {
            Some(
                cx.prompt_for_new_path(
                    known
                        .as_ref()
                        .and_then(|p| p.parent())
                        .unwrap_or(&self.options.output_dir),
                    Some(
                        &known
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "label.labelize.json".into()),
                    ),
                ),
            )
        } else {
            None
        };
        self.saving = true;
        cx.notify();
        cx.spawn_in(window,async move |this,cx| {
            let selected=if let Some(picker)=picker {match picker.await {Ok(Ok(path))=>Ok(path),Ok(Err(e))=>Err(e.to_string()),Err(e)=>Err(e.to_string())}} else {Ok(known.clone())};
            let path=match selected {
                Ok(Some(p))=>p,
                other=>{let _=this.update_in(cx,|this,_,cx| {this.saving=false;this.close_after_save=false;if let Err(e)=other {this.fail(e,cx);}cx.notify();});return;}
            };
            let check_path=path.clone();let is_original=known.as_ref().is_some_and(|p|session::canonical_path(p)==session::canonical_path(&path));
            let fingerprint=expected.clone();
            let conflict=cx.background_executor().spawn(async move {is_original && !session::source_matches(&check_path,fingerprint.as_deref())}).await;
            if conflict {
                let prompt=this.update_in(cx,|this,w,cx|w.prompt(PromptLevel::Warning,this.t("源文件已在外部更改或移动", "The source file changed or moved"),Some(this.t("本机草稿已保留。覆盖会替换磁盘上的文件；另存为可保留两份。", "Your local draft is retained. Overwrite replaces the disk file; Save as keeps both.")),&[PromptButton::cancel(this.t("取消", "Cancel")),PromptButton::new(this.t("另存为…", "Save as…")),PromptButton::new(this.t("覆盖", "Overwrite"))],cx));
                let answer=match prompt {Ok(p)=>p.await.ok(),Err(_)=>None};
                if answer!=Some(2) { let _=this.update_in(cx,|this,w,cx|{this.saving=false;if answer==Some(1) {this.save_as(true,w,cx);} else {this.close_after_save=false;}cx.notify();});return; }
            }
            let target=path.clone();let saved_hash=session::digest(&bytes);
            let result=cx.background_executor().spawn(async move {atomic_write(&target,&bytes)}).await;
            let _=this.update_in(cx,|this,_,cx| {
                this.saving=false;
                match result {
                    Ok(())=>{this.file=Some(session::canonical_path(&path));this.fingerprint=Some(saved_hash);if this.revision==revision {this.dirty=false;this.close_ready=this.close_after_save;}this.persist_version+=1;this.notice=Some(Ok(this.t("标签文件已保存。", "Label file saved.").into()));},
                    Err(e)=>this.fail(e,cx),
                }
                this.close_after_save=false;cx.notify();
            });
        }).detach();
    }
    pub fn request_tab_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_prompt || self.saving || self.loading || self.submitting {
            return;
        }
        if !self.dirty {
            self.close_ready = true;
            cx.notify();
            return;
        }
        self.close_prompt = true;
        let prompt = window.prompt(
            PromptLevel::Warning,
            self.t("保存对此标签的修改？", "Save changes to this label?"),
            Some(&self.title()),
            &[
                PromptButton::new(self.t("保存", "Save")),
                PromptButton::new(self.t("放弃", "Discard")),
                PromptButton::cancel(self.t("取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = prompt.await;
            let _ = this.update_in(cx, |this, w, cx| {
                this.close_prompt = false;
                match answer {
                    Ok(0) => {
                        this.close_after_save = true;
                        this.save(w, cx);
                    }
                    Ok(1) => this.close_ready = true,
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.t("输出目录", "Output folder").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = choice.await;
            let _ = this.update_in(cx, |this, _, cx| {
                match selected {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => {
                        this.options.output_dir = paths[0].clone();
                        this.persist_version += 1;
                    }
                    Ok(Err(e)) => this.fail(e, cx),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn pick_file(&mut self, purpose: u8, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(self.t("选择文件", "Choose file").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let path = match choice.await {
                Ok(Ok(Some(p))) => p.into_iter().next(),
                _ => None,
            };
            let Some(path) = path else {
                let _ = this.update_in(cx, |s, _, cx| {
                    s.loading = false;
                    cx.notify();
                });
                return;
            };
            let read = cx
                .background_executor()
                .spawn(async move { document::read_input(&path) })
                .await;
            let _ = this.update_in(cx, |s, w, cx| {
                s.loading = false;
                let result = (|| -> Result<()> {
                    let bytes = read?;
                    s.apply_fields(cx)?;
                    if purpose == 1 || purpose == 3 {
                        let image = labelize_printer_core::design::normalize_image(&bytes)?;
                        if purpose == 3 {
                            return s.edit_design(w, cx, move |d, i| {
                                let o = d.objects.get_mut(i).context("Select an image")?;
                                ensure!(
                                    o.kind == ObjectKind::Image && !o.style.locked,
                                    "Select an unlocked image"
                                );
                                o.image = image;
                                Ok(i)
                            });
                        }
                        ensure!(
                            s.design.as_ref().is_some_and(|d| d.objects.len() < 100),
                            "At most 100 objects"
                        );
                        s.remember();
                        if let Some(d) = &mut s.design {
                            let mut o = DesignObject::new(ObjectKind::Image);
                            o.image = image;
                            fit(&mut o, d);
                            d.objects.push(o);
                            s.selected = Some(d.objects.len() - 1);
                        }
                    } else {
                        ensure!(bytes.len() <= 32 * 1024 * 1024, "Font exceeds 32 MiB");
                        s.remember();
                        if let Some(d) = &mut s.design {
                            d.font = bytes;
                        }
                    }
                    s.changed(cx);
                    s.sync_form(w, cx);
                    Ok(())
                })();
                if let Err(e) = result {
                    s.fail(e, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn relink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(self.t("重新定位源文件", "Locate source file").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = choice.await;
            let _ = this.update_in(cx, |s, _, cx| {
                if let Ok(Ok(Some(paths))) = answer {
                    if let Some(path) = paths.first() {
                        s.file = Some(session::canonical_path(path));
                        s.persist_version += 1;
                        s.notice = Some(Ok(s
                            .t(
                                "路径已更新，草稿内容保持不变。",
                                "Path updated; draft content retained.",
                            )
                            .into()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn duplicate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.apply_fields(cx) {
            self.fail(e, cx);
            return;
        }
        if self.design.as_ref().is_none_or(|d| d.objects.len() >= 100) {
            return;
        }
        self.remember();
        if let (Some(d), Some(i)) = (&mut self.design, self.selected) {
            let mut o = d.objects[i].clone();
            let (width, height) = o.bounds_size();
            o.x = (o.x + 2.).min(d.width_mm - width);
            o.y = (o.y + 2.).min(d.height_mm - height);
            o.style.locked = false;
            d.objects.push(o);
            self.selected = Some(d.objects.len() - 1);
        }
        self.changed(cx);
        self.sync_form(window, cx);
    }
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_object().is_some_and(|o| o.style.locked) {
            return;
        }
        self.remember();
        if let (Some(d), Some(i)) = (&mut self.design, self.selected) {
            if i < d.objects.len() {
                d.objects.remove(i);
            }
            self.selected =
                (!d.objects.is_empty()).then_some(i.min(d.objects.len().saturating_sub(1)));
        }
        self.changed(cx);
        self.sync_form(window, cx);
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
            object.x += (d.objects.len() % 5) as f64 * 2.;
            object.y += (d.objects.len() % 5) as f64 * 2.;
            fit(&mut object, d);
            d.objects.push(object);
            self.selected = Some(d.objects.len() - 1);
        }
        self.changed(cx);
        self.sync_form(window, cx);
        self.focus.focus(window);
        self.render_preview(window, cx);
    }
    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.apply_fields(cx) {
            self.fail(e, cx);
            return;
        }
        self.selected = Some(index);
        self.persist_version += 1;
        self.sync_form(window, cx);
        self.focus.focus(window);
    }
    pub fn history(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.apply_fields(cx) {
            // Undo must also recover from an unfinished numeric entry.
            if !redo {
                self.sync_form(window, cx);
                self.preview_error = None;
                self.changed(cx);
            } else {
                self.fail(e, cx);
            }
            return;
        }
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
            self.selected = self.design.as_ref().and_then(|d| {
                (!d.objects.is_empty()).then_some(
                    self.selected
                        .unwrap_or(0)
                        .min(d.objects.len().saturating_sub(1)),
                )
            });
            self.changed(cx);
            self.sync_form(window, cx);
            self.render_preview(window, cx);
        }
    }
    fn object_name(&self, kind: ObjectKind) -> &'static str {
        match kind {
            ObjectKind::Text => self.t("文字", "Text"),
            ObjectKind::Code128 => "Code 128",
            ObjectKind::Code39 => "Code 39",
            ObjectKind::Ean13 => "EAN-13",
            ObjectKind::Ean8 => "EAN-8",
            ObjectKind::Upca => "UPC-A",
            ObjectKind::Itf => "ITF",
            ObjectKind::Qr => self.t("二维码", "QR code"),
            ObjectKind::DataMatrix => "Data Matrix",
            ObjectKind::Pdf417 => "PDF417",
            ObjectKind::Aztec => "Aztec",
            ObjectKind::Rectangle => self.t("矩形", "Rectangle"),
            ObjectKind::Ellipse => self.t("圆形 / 椭圆", "Ellipse"),
            ObjectKind::Line => self.t("线条", "Line"),
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
            .child(Input::new(&self.fields[index]).disabled(
                self.loading
                    || index >= 2 && self.selected_object().is_some_and(|o| o.style.locked),
            ))
    }
    fn selected_object(&self) -> Option<&DesignObject> {
        self.design
            .as_ref()
            .and_then(|d| self.selected.and_then(|i| d.objects.get(i)))
    }
    fn edit_design(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut DesignDocument, usize) -> Result<usize>,
    ) -> Result<()> {
        self.apply_fields(cx)?;
        let mut next = self.design.clone().context("Open a design")?;
        let selected = edit(&mut next, self.selected.context("Select an object")?)?;
        next.version = TEMPLATE_VERSION;
        next.validate(self.options.dpmm)?;
        if self.design.as_ref() != Some(&next) {
            self.remember();
            self.design = Some(next);
            self.selected = Some(selected);
            self.changed(cx);
            self.sync_form(window, cx);
        }
        Ok(())
    }
    fn change_style(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut labelize_printer_core::design::ObjectStyle),
    ) {
        let result = self.edit_design(window, cx, |d, i| {
            ensure!(
                !d.objects[i].style.locked,
                "Unlock this object before editing properties"
            );
            edit(&mut d.objects[i].style);
            Ok(i)
        });
        if let Err(e) = result {
            self.fail(e, cx);
        }
    }
    fn layer_flag(
        &mut self,
        index: usize,
        lock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.edit_design(window, cx, |d, _| {
            let style = &mut d.objects.get_mut(index).context("Select an object")?.style;
            if lock {
                style.locked = !style.locked;
            } else {
                style.visible = !style.visible;
            }
            Ok(index)
        });
        if let Err(e) = result {
            self.fail(e, cx);
        }
    }
    fn reorder_layer(&mut self, order: LayerOrder, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.edit_design(window, cx, |d, i| d.reorder(i, order));
        if let Err(e) = result {
            self.fail(e, cx);
        }
    }
    fn align_object(&mut self, alignment: Alignment, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.edit_design(window, cx, |d, i| {
            d.align(i, alignment)?;
            Ok(i)
        });
        if let Err(e) = result {
            self.fail(e, cx);
        }
    }
    fn scroll_drawer(&self, direction: f32, cx: &mut Context<Self>) {
        let scroll = if self.inspector_open {
            &self.inspector_scroll
        } else if self.layers_open {
            &self.layers_scroll
        } else {
            cx.propagate();
            return;
        };
        let mut offset = scroll.offset();
        offset.y = (offset.y + direction * scroll.bounds().size.height * 0.8)
            .clamp(-scroll.max_offset().height, px(0.));
        scroll.set_offset(offset);
        cx.notify();
    }
    fn finish_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.sync_form(window, cx);
            self.render_preview(window, cx);
        }
    }
    fn begin_drag(
        &mut self,
        index: usize,
        origin: Point<Pixels>,
        resize: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.apply_fields(cx) {
            self.fail(e, cx);
            return;
        }
        self.select(index, window, cx);
        self.focus.focus(window);
        if self.selected != Some(index) || self.selected_object().is_none_or(|o| o.style.locked) {
            return;
        }
        self.remember();
        if let Some(o) = self.selected_object() {
            let (width, height) = o.bounds_size();
            self.drag = Some(ObjectDrag {
                origin,
                x: o.x,
                y: o.y,
                width,
                height,
                resize,
            });
        }
    }
    fn nudge(&mut self, dx: f64, dy: f64, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.edit_design(window, cx, |d, i| {
            let o = &mut d.objects[i];
            ensure!(!o.style.locked, "Unlock this object before moving it");
            let (w, h) = o.bounds_size();
            o.x = (o.x + dx).clamp(0., (d.width_mm - w).max(0.));
            o.y = (o.y + dy).clamp(0., (d.height_mm - h).max(0.));
            Ok(i)
        });
        if let Err(e) = result {
            self.fail(e, cx);
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
fn editable_source(bytes: &[u8]) -> bool {
    bytes.len() <= 1024 * 1024
        && std::str::from_utf8(bytes).is_ok_and(|s| {
            s.chars()
                .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        })
}
fn section(title: &'static str) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(title)
}

impl Studio {
    fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .child(section(self.t("页面设置", "PAGE SETTINGS")))
            .child(
                field_row()
                    .child(self.field(0, self.t("宽度 · mm", "Width · mm")))
                    .child(self.field(1, self.t("高度 · mm", "Height · mm"))),
            )
            .child(row().children([8, 12].into_iter().map(|dpmm| {
                Button::new(("document-dpmm", dpmm as usize))
                    .small()
                    .label(format!("{dpmm} dpmm"))
                    .selected(self.options.dpmm == dpmm)
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.options.dpmm = dpmm;
                        s.changed(cx);
                    }))
            })));
        if self.design.is_none() {
            content = content
                .child(div().text_xs().text_color(rgb(MUTED)).child(self.t(
                    "文件声明的尺寸优先于此处缺省尺寸。",
                    "File dimensions take precedence over these defaults.",
                )))
                .child(
                    row().children(DocumentKind::LANGUAGES.into_iter().enumerate().map(
                        |(i, kind)| {
                            Button::new(("file-language", i))
                                .small()
                                .ghost()
                                .label(kind.name())
                                .selected(self.kind == kind)
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    s.kind = kind;
                                    s.changed(cx);
                                }))
                        },
                    )),
                );
        }
        if self.selected_object().is_some() {
            content = content
                .child(div().h(px(1.)).bg(rgb(BORDER)))
                .child(self.object_properties(cx));
        }
        content = content
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(section(self.t("导出文件", "EXPORT")))
            .child(
                row().children(
                    [(OutputFormat::Pdf, "PDF"), (OutputFormat::Png, "PNG")]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (format, label))| {
                            Button::new(("export-format", i))
                                .flex_1()
                                .label(label)
                                .selected(self.options.format == format)
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    if s.options.format == format {
                                        return;
                                    }
                                    let scale = s.options.output_scale();
                                    s.options.format = format;
                                    if s.options.output_scale() != scale {
                                        s.render_options_changed(cx);
                                    } else {
                                        s.persist_version += 1;
                                        cx.notify();
                                    }
                                }))
                        }),
                ),
            )
            .child(self.render_controls(cx))
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                if self.options.format == OutputFormat::Pdf {
                    self.t("全部页面合并为一个 PDF", "All pages in one PDF")
                } else {
                    self.t("每页保存为一张 PNG", "One PNG file per page")
                },
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(self.t("保存到", "SAVE TO")),
            )
            .child(
                div()
                    .text_sm()
                    .overflow_hidden()
                    .child(self.options.output_dir.to_string_lossy().into_owned()),
            )
            .child(
                Button::new("document-folder")
                    .small()
                    .icon(IconName::FolderOpen)
                    .label(self.t("更改目录…", "Choose folder…"))
                    .on_click(cx.listener(|s, _, w, cx| s.folder(w, cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(self.t("仅用于当前标签页", "Applies to this tab only")),
            );
        if self.file.as_ref().is_some_and(|p| !p.exists()) {
            content = content
                .child(div().text_xs().text_color(rgb(0xa22a34)).child(self.t(
                    "源文件路径已失效，草稿已保留。",
                    "Source file is missing. Your draft is retained.",
                )))
                .child(
                    Button::new("relink-source")
                        .small()
                        .label(self.t("重新定位…", "Locate file…"))
                        .on_click(cx.listener(|s, _, w, cx| s.relink(w, cx))),
                );
        }
        div()
            .id("document-inspector-scroll")
            .track_scroll(&self.inspector_scroll)
            .h_full()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::i18n::Text;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .child(section(self.language.text(Text::RenderMode)))
            .child(
                row().children(
                    [
                        (RenderMode::Raster, Text::RasterMode),
                        (RenderMode::Vector, Text::VectorMode),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (mode, label))| {
                        Button::new(("document-render-mode", i))
                            .small()
                            .flex_1()
                            .label(self.language.text(label))
                            .selected(self.options.render_mode == mode)
                            .on_click(cx.listener(move |s, _, _, cx| {
                                if s.options.render_mode != mode {
                                    s.options.render_mode = mode;
                                    s.render_options_changed(cx);
                                }
                            }))
                    }),
                ),
            )
            .child(
                Button::new("document-antialias")
                    .small()
                    .label(self.language.text(if self.options.antialias {
                        Text::AntialiasOn
                    } else {
                        Text::AntialiasOff
                    }))
                    .selected(self.options.antialias)
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.options.antialias = !s.options.antialias;
                        s.render_options_changed(cx);
                    })),
            )
            .when(
                self.options.render_mode == RenderMode::Vector
                    && self.options.format == OutputFormat::Png,
                |panel| {
                    panel
                        .child(section(self.language.text(Text::PngScale)))
                        .child(row().children((1..=4u32).map(|scale| {
                            Button::new(("document-png-scale", scale as usize))
                                .small()
                                .flex_1()
                                .label(format!("{scale}×"))
                                .selected(self.options.png_scale == scale)
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    if s.options.png_scale != scale {
                                        s.options.png_scale = scale;
                                        s.render_options_changed(cx);
                                    }
                                }))
                        })))
                },
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(
                        self.language
                            .text(if self.options.render_mode == RenderMode::Vector {
                                Text::VectorDescription
                            } else {
                                Text::AntialiasDescription
                            }),
                    ),
            )
    }
    fn layers(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(section(self.t("添加对象", "ADD OBJECT")));
        let mut tools = div().grid().grid_cols(2).gap_2();
        for (i, kind) in [
            ObjectKind::Text,
            ObjectKind::Rectangle,
            ObjectKind::Ellipse,
            ObjectKind::Line,
        ]
        .into_iter()
        .enumerate()
        {
            tools = tools.child(
                Button::new(("add-object", i))
                    .small()
                    .label(self.object_name(kind))
                    .on_click(cx.listener(move |s, _, w, cx| s.add(kind, w, cx))),
            );
        }
        content = content.child(tools).child(
            Button::new("add-image")
                .small()
                .icon(IconName::Plus)
                .label(self.t("图片…", "Image…"))
                .disabled(self.loading)
                .on_click(cx.listener(|s, _, w, cx| s.pick_file(1, w, cx))),
        );
        content = content
            .child(section(self.t("条码 / 二维码", "BARCODES")))
            .child(
                div().grid().grid_cols(2).gap_2().children(
                    ObjectKind::BARCODES
                        .into_iter()
                        .enumerate()
                        .map(|(i, kind)| {
                            Button::new(("add-barcode", i))
                                .small()
                                .label(self.object_name(kind))
                                .on_click(cx.listener(move |s, _, w, cx| s.add(kind, w, cx)))
                        }),
                ),
            )
            .child(section(self.t("对象列表", "LAYERS")));
        if let Some(d) = &self.design {
            for (i, o) in d.objects.iter().enumerate().rev() {
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .child(
                            Button::new(("layer", i))
                                .ghost()
                                .small()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .justify_start()
                                .selected(self.selected == Some(i))
                                .label(if o.style.name.is_empty() {
                                    format!("{:02}  {}", i + 1, self.object_name(o.kind))
                                } else {
                                    o.style.name.clone()
                                })
                                .tooltip(self.object_name(o.kind))
                                .on_click(cx.listener(move |s, _, w, cx| s.select(i, w, cx))),
                        )
                        .child(
                            Button::new(("layer-visible", i))
                                .small()
                                .ghost()
                                .icon(if o.style.visible {
                                    IconName::Eye
                                } else {
                                    IconName::EyeOff
                                })
                                .tooltip(self.t("显示 / 隐藏", "Show / hide"))
                                .on_click(
                                    cx.listener(move |s, _, w, cx| s.layer_flag(i, false, w, cx)),
                                ),
                        )
                        .child(
                            Button::new(("layer-lock", i))
                                .small()
                                .ghost()
                                .label(if o.style.locked {
                                    self.t("解锁", "Unlock")
                                } else {
                                    self.t("锁定", "Lock")
                                })
                                .selected(o.style.locked)
                                .tooltip(self.t("锁定 / 解锁", "Lock / unlock"))
                                .on_click(
                                    cx.listener(move |s, _, w, cx| s.layer_flag(i, true, w, cx)),
                                ),
                        ),
                );
            }
        }
        content = content
            .child(
                Button::new("import-font")
                    .small()
                    .ghost()
                    .label(self.t("导入字体…", "Import font…"))
                    .disabled(self.loading)
                    .on_click(cx.listener(|s, _, w, cx| s.pick_file(2, w, cx))),
            )
            .child(div().text_xs().text_color(rgb(MUTED)).child(self.t(
                "中文请导入含对应字形的字体。",
                "Import a font for CJK glyphs.",
            )));
        div()
            .id("document-layers-scroll")
            .track_scroll(&self.layers_scroll)
            .h_full()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
    fn preview_canvas(&self, width: f32, height: f32, cx: &mut Context<Self>) -> AnyElement {
        let mut area = div()
            .id("label-canvas-scroll")
            .size_full()
            .overflow_scroll()
            .track_scroll(&self.canvas_scroll);
        if let Some(display) = &self.display {
            let image = &display.image;
            let page = &display.page;
            let padding = if height < 220. { 16. } else { 48. };
            let fit = ((width - padding).max(1.) / page.width_mm as f32)
                .min((height - padding).max(1.) / page.height_mm as f32);
            let scale = self.zoom.map_or(fit, |z| z * 96. / 25.4);
            let w = page.width_mm as f32 * scale;
            let h = page.height_mm as f32 * scale;
            let mut canvas = div()
                .id("design-canvas")
                .relative()
                .flex_shrink_0()
                .w(px(w))
                .h(px(h))
                .bg(rgb(0xffffff))
                .shadow_md()
                .child(img(image.clone()).size_full());
            if let Some(d) = &self.design {
                for (i, o) in d
                    .objects
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.style.visible)
                {
                    let (object_width, object_height) = o.bounds_size();
                    canvas = canvas.child(
                        div()
                            .id(("canvas-object", i))
                            .absolute()
                            .left(px(o.x as f32 * scale))
                            .top(px(o.y as f32 * scale))
                            .w(px(object_width as f32 * scale))
                            .h(px(object_height as f32 * scale))
                            .when(!o.style.locked, |d| d.cursor_move())
                            .border_1()
                            .border_color(if self.selected == Some(i) {
                                rgba(0x315efbff)
                            } else {
                                rgba(0x315efb00)
                            })
                            .when(self.selected == Some(i) && !o.style.locked, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .size(px(6.))
                                        .top(px(-3.))
                                        .left(px(-3.))
                                        .bg(rgb(0xffffff))
                                        .border_1()
                                        .border_color(rgb(ACCENT)),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .size(px(6.))
                                        .bottom(px(-3.))
                                        .right(px(-3.))
                                        .bg(rgb(0xffffff))
                                        .border_1()
                                        .border_color(rgb(ACCENT))
                                        .cursor_crosshair()
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |s, e: &MouseDownEvent, w, cx| {
                                                s.begin_drag(i, e.position, true, w, cx);
                                                cx.stop_propagation();
                                            }),
                                        ),
                                )
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |s, e: &MouseDownEvent, w, cx| {
                                    s.begin_drag(i, e.position, false, w, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                    );
                }
            }
            canvas = canvas
                .on_mouse_move(cx.listener(move |s, e: &MouseMoveEvent, w, cx| {
                    if let Some(drag) = s.drag {
                        if e.pressed_button != Some(MouseButton::Left) {
                            s.finish_drag(w, cx);
                            return;
                        }
                        if let (Some(d), Some(i)) = (&mut s.design, s.selected) {
                            let o = &mut d.objects[i];
                            let dx = f64::from(f32::from(e.position.x - drag.origin.x) / scale);
                            let dy = f64::from(f32::from(e.position.y - drag.origin.y) / scale);
                            if drag.resize {
                                let width = (drag.width + dx).clamp(1., (d.width_mm - o.x).max(1.));
                                let height =
                                    (drag.height + dy).clamp(1., (d.height_mm - o.y).max(1.));
                                (o.width, o.height) = if matches!(o.style.rotation, 90 | 270) {
                                    (height, width)
                                } else {
                                    (width, height)
                                };
                                o.stroke_mm = o.stroke_mm.min(o.width.min(o.height));
                                o.style.corner_radius_mm =
                                    o.style.corner_radius_mm.min(o.width.min(o.height) / 2.);
                            } else {
                                o.x = (drag.x + dx).clamp(0., (d.width_mm - drag.width).max(0.));
                                o.y = (drag.y + dy).clamp(0., (d.height_mm - drag.height).max(0.));
                            }
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
                );
            area = area.child(
                div()
                    .min_w_full()
                    .min_h_full()
                    .w(px(w + padding))
                    .h(px(h + padding))
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(padding / 2.))
                    .child(canvas),
            );
        } else {
            area = area.child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .text_color(rgb(MUTED))
                    .child(gpui_component::Icon::new(IconName::File).size(px(32.)))
                    .child(div().text_sm().child(
                        if self.rendering.is_running() || self.image_loading {
                            self.t("正在生成预览…", "Rendering preview…")
                        } else {
                            self.t("预览将在这里显示", "Your preview appears here")
                        },
                    )),
            );
        }
        area.into_any_element()
    }
}
impl Render for Studio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let height = f32::from(window.viewport_size().height);
        let narrow = width < 700.;
        let short = height < 500.;
        let wide = width >= 1000.;
        let inspector_width = 260.;
        let layers_width = 224.;
        let sidebar = if width >= 1000. { 188. } else { 56. };
        let canvas_width = width
            - sidebar
            - if narrow { 0. } else { inspector_width }
            - if wide && self.design.is_some() {
                layers_width
            } else {
                0.
            };
        let page_count = self.preview.as_ref().map_or(0, |p| p.pages.len());
        let mut toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .h(px(if short { 36. } else { 44. }))
            .px_3()
            .flex_shrink_0()
            .bg(rgb(0xffffff))
            .border_b_1()
            .border_color(rgb(BORDER));
        let mut modes = div().flex().items_center().gap_1();
        if self.design.is_some() {
            if !wide {
                modes = modes.child(
                    Button::new("toggle-layers")
                        .small()
                        .ghost()
                        .icon(IconName::GalleryVerticalEnd)
                        .selected(self.layers_open)
                        .on_click(cx.listener(|s, _, w, cx| {
                            s.focus.focus(w);
                            s.layers_open = !s.layers_open;
                            s.inspector_open = false;
                            cx.notify();
                        })),
                );
            }
            modes = modes
                .child(
                    Button::new("undo-design")
                        .small()
                        .ghost()
                        .icon(IconName::Undo2)
                        .tooltip(self.t("撤销", "Undo"))
                        .disabled(self.undo.is_empty())
                        .on_click(cx.listener(|s, _, w, cx| s.history(false, w, cx))),
                )
                .child(
                    Button::new("redo-design")
                        .small()
                        .ghost()
                        .icon(IconName::Redo2)
                        .tooltip(self.t("重做", "Redo"))
                        .disabled(self.redo.is_empty())
                        .on_click(cx.listener(|s, _, w, cx| s.history(true, w, cx))),
                );
        } else {
            modes = modes
                .child(
                    Button::new("preview-mode")
                        .small()
                        .ghost()
                        .label(self.t("预览", "Preview"))
                        .selected(!self.source_mode)
                        .on_click(cx.listener(|s, _, w, cx| {
                            s.focus.focus(w);
                            s.source_mode = false;
                            s.persist_version += 1;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("source-mode")
                        .small()
                        .ghost()
                        .label(self.t("源代码", "Source"))
                        .selected(self.source_mode)
                        .on_click(cx.listener(|s, _, w, cx| {
                            s.source_mode = true;
                            s.focus_editor(w, cx);
                            s.persist_version += 1;
                            cx.notify();
                        })),
                );
        }
        let zoom = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("zoom-out")
                    .small()
                    .ghost()
                    .icon(IconName::Minus)
                    .tooltip(self.t("缩小", "Zoom out"))
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.zoom = Some((s.zoom.unwrap_or(1.) - 0.1).max(0.1));
                        s.persist_version += 1;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("zoom-fit")
                    .small()
                    .ghost()
                    .label(self.zoom.map_or_else(
                        || self.t("适合", "Fit").into(),
                        |z| format!("{:.0}%", z * 100.),
                    ))
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.zoom = None;
                        s.canvas_scroll.set_offset(Point::default());
                        s.persist_version += 1;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("zoom-in")
                    .small()
                    .ghost()
                    .icon(IconName::Plus)
                    .tooltip(self.t("放大", "Zoom in"))
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.zoom = Some((s.zoom.unwrap_or(1.) + 0.1).min(4.));
                        s.persist_version += 1;
                        cx.notify();
                    })),
            )
            .when(narrow, |d| {
                d.child(
                    Button::new("toggle-inspector")
                        .small()
                        .ghost()
                        .icon(IconName::Settings2)
                        .selected(self.inspector_open)
                        .tooltip(self.t("属性与导出", "Properties & export"))
                        .on_click(cx.listener(|s, _, w, cx| {
                            s.focus.focus(w);
                            s.inspector_open = !s.inspector_open;
                            s.layers_open = false;
                            cx.notify();
                        })),
                )
            });
        toolbar = toolbar.child(modes).child(zoom);
        let canvas = if self.source_mode && self.design.is_none() {
            if self.editable {
                div()
                    .size_full()
                    .p_4()
                    .bg(rgb(0xffffff))
                    .child(Input::new(&self.editor).size_full())
                    .into_any_element()
            } else {
                div().size_full().p_6().text_sm().text_color(rgb(MUTED)).child(self.t("二进制、控制字节或超过 1 MiB 的文件只读。预览和导出保留原始字节。", "Binary/control data and files over 1 MiB are read-only. Preview and export preserve the original bytes.")).into_any_element()
            }
        } else {
            self.preview_canvas(
                if self.canvas_size.width > px(0.) {
                    f32::from(self.canvas_size.width)
                } else {
                    canvas_width
                },
                if self.canvas_size.height > px(0.) {
                    f32::from(self.canvas_size.height)
                } else {
                    height - 350.
                },
                cx,
            )
        };
        let mut center = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(toolbar);
        let weak = cx.weak_entity();
        center = center.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(canvas)
                .child(
                    gpui::canvas(
                        move |bounds, _, cx| {
                            cx.defer(move |cx| {
                                let _ = weak.update(cx, |s, cx| {
                                    if s.canvas_size != bounds.size {
                                        s.canvas_size = bounds.size;
                                        cx.notify();
                                    }
                                });
                            });
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .when(self.stale || self.image_loading, |area| {
                    area.child(
                        div()
                            .id("preview-status")
                            .absolute()
                            .top_2()
                            .left_2()
                            .right_2()
                            .max_h(px(80.))
                            .overflow_y_scroll()
                            .rounded_md()
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(rgb(if self.preview_error.is_some() {
                                0xa22a34
                            } else {
                                MUTED
                            }))
                            .bg(rgb(if self.preview_error.is_some() {
                                0xfff0f0
                            } else {
                                0xeef3ff
                            }))
                            .child(if let Some(e) = &self.preview_error {
                                format!("{} · {e}", self.t("预览未更新", "Preview not updated"))
                            } else {
                                self.t("正在更新预览…", "Updating preview…").into()
                            }),
                    )
                }),
        );
        if page_count > 0 && !self.source_mode {
            let mut pages = div()
                .h(px(if short { 32. } else { 84. }))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .px_2()
                .border_t_1()
                .border_color(rgb(BORDER))
                .bg(rgb(0xfafdff))
                .child(
                    Button::new("previous-page")
                        .small()
                        .ghost()
                        .icon(IconName::ChevronLeft)
                        .disabled(self.page == 0)
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.page = s.page.saturating_sub(1);
                            s.persist_version += 1;
                            s.show_page(cx);
                        })),
                );
            for (i, image) in self
                .display
                .iter()
                .flat_map(|d| &d.thumbnails)
                .filter(|(i, _)| *i < page_count)
            {
                let i = *i;
                if short {
                    pages = pages.child(
                        Button::new(("compact-page", i))
                            .xsmall()
                            .ghost()
                            .label((i + 1).to_string())
                            .selected(i == self.page)
                            .on_click(cx.listener(move |s, _, _, cx| {
                                s.page = i;
                                s.persist_version += 1;
                                s.show_page(cx);
                            })),
                    );
                    continue;
                }
                pages = pages.child(
                    div()
                        .id(("page-thumbnail", i))
                        .cursor_pointer()
                        .w(px(40.))
                        .h(px(68.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .p_1()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if i == self.page { ACCENT } else { BORDER }))
                        .bg(rgb(0xffffff))
                        .child(
                            img(image.clone())
                                .w(px(29.))
                                .h(px(44.))
                                .object_fit(ObjectFit::Contain),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(MUTED))
                                .child((i + 1).to_string()),
                        )
                        .on_click(cx.listener(move |s, _, _, cx| {
                            s.page = i;
                            s.persist_version += 1;
                            s.show_page(cx);
                        })),
                );
            }
            pages = pages.child(
                Button::new("next-page")
                    .small()
                    .ghost()
                    .icon(IconName::ChevronRight)
                    .disabled(self.page + 1 >= page_count)
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.page += 1;
                        s.persist_version += 1;
                        s.show_page(cx);
                    })),
            );
            center = center.child(pages);
        }
        let mut content = div()
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden();
        if wide && self.design.is_some() {
            content = content.child(
                div()
                    .w(px(layers_width))
                    .flex_shrink_0()
                    .h_full()
                    .bg(rgb(0xffffff))
                    .border_r_1()
                    .border_color(rgb(BORDER))
                    .child(self.layers(cx)),
            );
        }
        content = content.child(center);
        if !narrow {
            content = content.child(
                div()
                    .w(px(inspector_width))
                    .h_full()
                    .flex_shrink_0()
                    .border_l_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(0xffffff))
                    .child(self.inspector(cx)),
            );
        }
        if narrow && self.inspector_open || !wide && self.layers_open {
            let inspector = self.inspector_open;
            content = content
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(rgba(0x17243b22))
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|s, _, w, cx| {
                                s.focus.focus(w);
                                s.inspector_open = false;
                                s.layers_open = false;
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .w(px(280.))
                        .bg(rgb(0xffffff))
                        .shadow_lg()
                        .occlude()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .px_3()
                                .py_2()
                                .border_b_1()
                                .border_color(rgb(BORDER))
                                .child(section(if inspector {
                                    self.t("属性与导出", "PROPERTIES & EXPORT")
                                } else {
                                    self.t("对象列表", "LAYERS")
                                }))
                                .child(
                                    Button::new("close-inspector")
                                        .small()
                                        .ghost()
                                        .icon(IconName::Close)
                                        .on_click(cx.listener(|s, _, w, cx| {
                                            s.focus.focus(w);
                                            s.inspector_open = false;
                                            s.layers_open = false;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(div().flex_1().min_h_0().child(if inspector {
                            self.inspector(cx)
                        } else {
                            self.layers(cx)
                        })),
                );
        }
        let mut root = div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|s, _: &ScrollPageDown, _, cx| s.scroll_drawer(-1., cx)))
            .on_action(cx.listener(|s, _: &ScrollPageUp, _, cx| s.scroll_drawer(1., cx)))
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && (s.inspector_open || s.layers_open) {
                    s.inspector_open = false;
                    s.layers_open = false;
                    s.focus.focus(w);
                    cx.stop_propagation();
                    cx.notify();
                }
                if s.focus.is_focused(w)
                    && s.design.is_some()
                    && s.selected.is_some()
                    && !e.keystroke.modifiers.control
                    && !e.keystroke.modifiers.platform
                    && !e.keystroke.modifiers.alt
                {
                    let step = if e.keystroke.modifiers.shift { 1. } else { 0.1 };
                    let direction = match e.keystroke.key.as_str() {
                        "left" => Some((-step, 0.)),
                        "right" => Some((step, 0.)),
                        "up" => Some((0., -step)),
                        "down" => Some((0., step)),
                        _ => None,
                    };
                    if let Some((dx, dy)) = direction {
                        s.nudge(dx, dy, w, cx);
                        cx.stop_propagation();
                    } else if matches!(e.keystroke.key.as_str(), "backspace" | "delete") {
                        s.delete(w, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(content);
        if let Some(notice) = &self.notice {
            let text = match notice {
                Ok(s) | Err(s) => s,
            };
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .bg(rgb(if notice.is_err() { 0xfff0f0 } else { 0xedf8f3 }))
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_h(px(if short { 44. } else { 100. }))
                            .id("document-notice-scroll")
                            .overflow_y_scroll()
                            .child(text.clone()),
                    )
                    .when(!self.outputs.is_empty(), |d| {
                        d.child(
                            Button::new("open-output")
                                .small()
                                .ghost()
                                .label(self.t("打开", "Open"))
                                .on_click(cx.listener(|s, _, _, cx| {
                                    if let Some(path) = s.outputs.first() {
                                        if let Err(e) = open::that_detached(path) {
                                            s.fail(e, cx);
                                        }
                                    }
                                })),
                        )
                    })
                    .child(
                        Button::new("dismiss-document-notice")
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
        root.child(
            div()
                .h(px(26.))
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .flex_shrink_0()
                .border_t_1()
                .border_color(rgb(BORDER))
                .bg(rgb(0xffffff))
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child(format!(
                    "{} · {}",
                    self.kind.name(),
                    self.preview
                        .as_ref()
                        .and_then(|p| p.pages.get(self.page))
                        .map_or_else(
                            || format!(
                                "{} × {} mm",
                                number(self.options.width_mm),
                                number(self.options.height_mm)
                            ),
                            |p| format!(
                                "{} × {} mm · {} × {} px",
                                number(p.width_mm),
                                number(p.height_mm),
                                p.width,
                                p.height
                            )
                        )
                ))
                .child(format!(
                    "{} / {}",
                    if page_count == 0 { 0 } else { self.page + 1 },
                    page_count
                )),
        )
    }
}
