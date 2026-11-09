use super::*;
use labelize_printer_core::design::ObjectStyle;

impl Studio {
    fn style_choices<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        selected: T,
        options: impl IntoIterator<Item = (T, &'static str)>,
        change: fn(&mut ObjectStyle, T),
        cx: &mut Context<Self>,
    ) -> Div {
        row().children(options.into_iter().enumerate().map(|(i, (value, title))| {
            Button::new((id, i))
                .small()
                .label(title)
                .selected(selected == value)
                .disabled(self.selected_object().is_some_and(|o| o.style.locked))
                .on_click(cx.listener(move |s, _, w, cx| {
                    s.change_style(w, cx, |style| change(style, value))
                }))
        }))
    }
    pub(super) fn object_properties(&self, cx: &mut Context<Self>) -> Div {
        let Some(o) = self.selected_object() else {
            return div();
        };
        let style = &o.style;
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(section(self.t("对象属性", "OBJECT PROPERTIES")))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.object_name(o.kind)),
            )
            .child(self.field(9, self.t("名称", "Name")))
            .child(
                row()
                    .child(
                        Button::new("object-visible")
                            .small()
                            .icon(if style.visible {
                                IconName::Eye
                            } else {
                                IconName::EyeOff
                            })
                            .label(if style.visible {
                                self.t("可见", "Visible")
                            } else {
                                self.t("已隐藏", "Hidden")
                            })
                            .on_click(cx.listener(|s, _, w, cx| {
                                if let Some(i) = s.selected {
                                    s.layer_flag(i, false, w, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("object-lock")
                            .small()
                            .selected(style.locked)
                            .label(if style.locked {
                                self.t("解锁", "Unlock")
                            } else {
                                self.t("锁定", "Lock")
                            })
                            .on_click(cx.listener(|s, _, w, cx| {
                                if let Some(i) = s.selected {
                                    s.layer_flag(i, true, w, cx);
                                }
                            })),
                    ),
            )
            .when(style.locked, |d| {
                d.child(div().text_xs().text_color(rgb(MUTED)).child(self.t(
                    "解锁后可移动或编辑此对象。",
                    "Unlock to move or edit this object.",
                )))
            })
            .child(
                field_row()
                    .child(self.field(2, "X · mm"))
                    .child(self.field(3, "Y · mm")),
            )
            .child(
                field_row()
                    .child(self.field(4, self.t("宽度 · mm", "Width · mm")))
                    .child(self.field(5, self.t("高度 · mm", "Height · mm"))),
            )
            .child(section(self.t("顺时针旋转", "ROTATION")))
            .child(row().children([0, 90, 180, 270].into_iter().map(|angle| {
                Button::new(("object-rotation", angle as usize))
                    .small()
                    .label(format!("{angle}°"))
                    .selected(style.rotation == angle)
                    .disabled(style.locked)
                    .on_click(cx.listener(move |s, _, w, cx| {
                        let result = s.edit_design(w, cx, |d, i| {
                            d.rotate(i, angle)?;
                            Ok(i)
                        });
                        if let Err(e) = result {
                            s.fail(e, cx);
                        }
                    }))
            })));
        if o.kind.has_content() {
            content = content.child(self.field(8, self.t("内容", "Content")));
        }
        if o.kind == ObjectKind::Text
            || (o.kind.is_barcode() && style.human_readable && !o.kind.is_matrix())
        {
            content = content
                .child(self.field(6, self.t("字号 · mm", "Font size · mm")))
                .child(section(self.t("字体", "FONT")))
                .child(self.style_choices(
                    "object-font",
                    style.font,
                    [
                        (TextFont::Document, self.t("文档字体", "Document")),
                        (TextFont::Monospace, self.t("等宽", "Mono")),
                        (TextFont::BoldMonospace, self.t("等宽粗体", "Mono bold")),
                        (
                            TextFont::CondensedBold,
                            self.t("紧凑粗体", "Condensed bold"),
                        ),
                    ],
                    |s, v| s.font = v,
                    cx,
                ));
        }
        if o.kind == ObjectKind::Text {
            content = content
                .child(self.style_choices(
                    "text-wrap",
                    style.wrap,
                    [
                        (false, self.t("手动换行", "Manual breaks")),
                        (true, self.t("自动换行", "Wrap text")),
                    ],
                    |s, v| s.wrap = v,
                    cx,
                ))
                .child(self.field(10, self.t("行距 · 倍数", "Line spacing · factor")));
        }
        if o.kind.is_shape() {
            content = content.child(self.field(7, self.t("线宽 · mm", "Stroke · mm")));
            if o.kind != ObjectKind::Line {
                content = content.child(self.style_choices(
                    "shape-filled",
                    style.filled,
                    [
                        (false, self.t("描边", "Outline")),
                        (true, self.t("实心", "Filled")),
                    ],
                    |s, v| s.filled = v,
                    cx,
                ));
            }
            if o.kind == ObjectKind::Rectangle {
                content =
                    content.child(self.field(11, self.t("圆角半径 · mm", "Corner radius · mm")));
            }
            if o.kind == ObjectKind::Line {
                content = content.child(self.style_choices(
                    "line-direction",
                    style.line_direction,
                    [
                        (LineDirection::Horizontal, self.t("水平", "Horizontal")),
                        (LineDirection::Vertical, self.t("垂直", "Vertical")),
                        (LineDirection::Down, "↘"),
                        (LineDirection::Up, "↗"),
                    ],
                    |s, v| s.line_direction = v,
                    cx,
                ));
            }
        }
        if o.kind == ObjectKind::Image {
            content = content
                .child(self.style_choices(
                    "image-fit",
                    style.image_fit,
                    [
                        (ImageFit::Contain, self.t("完整显示", "Contain")),
                        (ImageFit::Cover, self.t("裁切填充", "Cover")),
                        (ImageFit::Stretch, self.t("拉伸", "Stretch")),
                    ],
                    |s, v| s.image_fit = v,
                    cx,
                ))
                .child(self.field(16, self.t("黑白阈值 · 0–255", "Threshold · 0–255")))
                .child(
                    Button::new("replace-image")
                        .small()
                        .label(self.t("替换图片…", "Replace image…"))
                        .disabled(style.locked || self.loading)
                        .on_click(cx.listener(|s, _, w, cx| s.pick_file(3, w, cx))),
                );
        }
        if o.kind.is_barcode() {
            content = content
                .child(section(self.t("条码参数", "BARCODE")))
                .child(self.field(12, self.t("模块宽度 · mm", "Module width · mm")))
                .child(self.field(13, self.t("静区 · 模块数", "Quiet zone · modules")))
                .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                    "{} {}",
                    self.t("0 表示自动；静区最少", "0 = automatic; minimum quiet zone:"),
                    o.kind.minimum_quiet()
                )))
                .child(div().text_xs().text_color(rgb(MUTED)).child(self.t(
                    "模块宽度按当前分辨率取整到像素。",
                    "Module width rounds to whole dots at this resolution.",
                )));
            if !o.kind.is_matrix() {
                content = content.child(self.style_choices(
                    "barcode-caption",
                    style.human_readable,
                    [
                        (false, self.t("隐藏文字", "Hide caption")),
                        (true, self.t("显示文字", "Show caption")),
                    ],
                    |s, v| s.human_readable = v,
                    cx,
                ));
            }
            match o.kind {
                ObjectKind::Qr => {
                    content = content
                        .child(section(self.t("纠错级别", "ERROR CORRECTION")))
                        .child(self.style_choices(
                            "qr-ecc",
                            style.qr_ecc,
                            [
                                (QrEcc::L, "L"),
                                (QrEcc::M, "M"),
                                (QrEcc::Q, "Q"),
                                (QrEcc::H, "H"),
                            ],
                            |s, v| s.qr_ecc = v,
                            cx,
                        ));
                }
                ObjectKind::DataMatrix => {
                    content = content.child(self.style_choices(
                        "matrix-shape",
                        style.matrix_shape,
                        [
                            (MatrixShape::Square, self.t("方形", "Square")),
                            (MatrixShape::Rectangle, self.t("矩形", "Rectangular")),
                        ],
                        |s, v| s.matrix_shape = v,
                        cx,
                    ));
                }
                ObjectKind::Pdf417 => {
                    content = content
                        .child(self.field(14, self.t("列数 · 0 自动", "Columns · 0 auto")))
                        .child(self.field(15, self.t("纠错级别 · 0–8", "Error correction · 0–8")));
                }
                ObjectKind::Ean13 | ObjectKind::Ean8 | ObjectKind::Upca => {
                    content = content.child(div().text_xs().text_color(rgb(MUTED)).child(self.t(
                        "可输入完整号码或省略最后一位校验位。",
                        "Enter all digits, or omit the final check digit.",
                    )));
                }
                ObjectKind::Itf => {
                    content = content.child(div().text_xs().text_color(rgb(MUTED)).child(self.t(
                        "输入偶数位数字；不会自动补位。",
                        "Use an even number of digits; no automatic padding.",
                    )));
                }
                _ => {}
            }
        }
        if o.kind == ObjectKind::Text || o.kind == ObjectKind::Image || o.kind.is_barcode() {
            content = content
                .child(section(self.t("内容对齐", "CONTENT ALIGNMENT")))
                .child(self.style_choices(
                    "content-horizontal",
                    style.horizontal,
                    [
                        (HorizontalAlign::Left, self.t("左", "Left")),
                        (HorizontalAlign::Center, self.t("居中", "Center")),
                        (HorizontalAlign::Right, self.t("右", "Right")),
                    ],
                    |s, v| s.horizontal = v,
                    cx,
                ));
            if !o.kind.is_barcode() || o.kind.is_matrix() {
                content = content.child(self.style_choices(
                    "content-vertical",
                    style.vertical,
                    [
                        (VerticalAlign::Top, self.t("上", "Top")),
                        (VerticalAlign::Middle, self.t("居中", "Middle")),
                        (VerticalAlign::Bottom, self.t("下", "Bottom")),
                    ],
                    |s, v| s.vertical = v,
                    cx,
                ));
            }
        }
        content = content
            .child(section(self.t("对象背景", "BACKGROUND")))
            .child(self.style_choices(
                "object-background",
                style.background,
                [
                    (Background::Transparent, self.t("透明", "Transparent")),
                    (Background::White, self.t("白底", "White")),
                    (Background::Black, self.t("反白", "Black")),
                ],
                |s, v| s.background = v,
                cx,
            ))
            .child(section(self.t("相对标签对齐", "ALIGN TO LABEL")))
            .child(
                row().children(
                    [
                        (Alignment::Left, self.t("左", "Left")),
                        (Alignment::Center, self.t("水平居中", "H center")),
                        (Alignment::Right, self.t("右", "Right")),
                        (Alignment::Top, self.t("上", "Top")),
                        (Alignment::Middle, self.t("垂直居中", "V center")),
                        (Alignment::Bottom, self.t("下", "Bottom")),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (alignment, title))| {
                        Button::new(("align-object", i))
                            .small()
                            .label(title)
                            .disabled(style.locked)
                            .on_click(
                                cx.listener(move |s, _, w, cx| s.align_object(alignment, w, cx)),
                            )
                    }),
                ),
            )
            .child(section(self.t("图层顺序", "LAYER ORDER")))
            .child(
                row().children(
                    [
                        (LayerOrder::Front, self.t("置顶", "To front")),
                        (LayerOrder::Raise, self.t("上移", "Forward")),
                        (LayerOrder::Lower, self.t("下移", "Backward")),
                        (LayerOrder::Back, self.t("置底", "To back")),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (order, title))| {
                        Button::new(("layer-order", i))
                            .small()
                            .label(title)
                            .disabled(style.locked)
                            .on_click(cx.listener(move |s, _, w, cx| s.reorder_layer(order, w, cx)))
                    }),
                ),
            )
            .child(
                row()
                    .child(
                        Button::new("duplicate-object")
                            .small()
                            .icon(IconName::Copy)
                            .label(self.t("复制", "Duplicate"))
                            .on_click(cx.listener(|s, _, w, cx| s.duplicate(w, cx))),
                    )
                    .child(
                        Button::new("delete-object")
                            .small()
                            .ghost()
                            .icon(IconName::Delete)
                            .label(self.t("删除", "Delete"))
                            .disabled(style.locked)
                            .on_click(cx.listener(|s, _, w, cx| s.delete(w, cx))),
                    ),
            );
        content
    }
}
