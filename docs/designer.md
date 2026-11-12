# Label designer

[README](../README.md) · [Architecture](architecture.md) · [Acceptance](acceptance.md)

This guide describes the expanded designer in **Labelize Studio v0.4.0**. Existing Labelize Printer templates remain compatible.

Choose **Workspace → New design**, or open the [showcase template](../examples/designer-showcase.labelize.json). Objects are listed from front to back. Select an object to see the properties appropriate to its type. Preview and PDF/PNG export use the same rendering rules.

## Objects and properties

| Object | Properties |
| --- | --- |
| Text | Multiline content; document, monospace, bold monospace, or condensed bold font; size; manual or automatic wrapping; line spacing; horizontal and vertical alignment. |
| Rectangle | Stroke, outline or solid fill, and corner radius. |
| Ellipse / circle | Stroke and outline or solid fill. Equal width and height produce a circle. |
| Line | Stroke and horizontal, vertical, or either diagonal direction within its bounds. |
| Image | Embedded PNG/JPEG; replace image; contain, cover, or stretch; horizontal/vertical alignment; monochrome threshold. |
| Code 128, Code 39, EAN-13, EAN-8, UPC-A, ITF | Content, module width, quiet zone, horizontal alignment, optional human-readable caption and caption font/size. |
| QR | Content, module width, quiet zone, alignment, and L/M/Q/H error correction. |
| Data Matrix | Content, module width, quiet zone, alignment, and square or rectangular symbol selection. |
| PDF417 | Content, module width, quiet zone, alignment, columns (0 for automatic), and error correction 0–8. |
| Aztec | Content, module width, quiet zone, and alignment. |

Every object has a name, position, size, visibility, lock, clockwise quarter-turn rotation, and background. **Align to label** moves the object to an edge or center of the page. **Content alignment** positions text, images, or symbols within the object's own bounds. Width and height describe the object before rotation; a 90° or 270° turn swaps its occupied width and height. Rotation moves an object inward if necessary, and rejects a rotated object that cannot fit the page.

Transparent backgrounds preserve objects underneath. White backgrounds cover the full object rectangle; black backgrounds invert the object's pixels over an opaque black rectangle. New barcodes use white backgrounds to protect their quiet zones. A hidden object stays editable in the layer list but does not appear in preview or export. A locked object can be selected, hidden, unlocked, or duplicated, but cannot be moved, resized, reordered, deleted, or edited. Duplicating creates an unlocked copy.

## Editing

- Drag an unlocked object to move it; drag its bottom-right handle to resize. The object stays within the label. Resizing changes the bounds, not the text's font size.
- With canvas focus, arrows move 0.1 mm and Shift+arrows move 1 mm. Delete/Backspace removes the selected unlocked object. These keys keep their normal behavior inside text inputs.
- Use Undo/Redo in the canvas toolbar or Cmd/Ctrl+Z and Cmd/Ctrl+Shift+Z outside text inputs. History belongs to the current tab and covers style changes, layer operations, and object edits.
- Use **To front**, **Forward**, **Backward**, or **To back** to change overlap. Ordering follows the visible layer list.
- Below 1000 logical pixels, the toolbar's object-list button opens the tools/layers panel. Below 700, properties open in a separate drawer. Each panel scrolls independently; Page Up/Down scrolls the open drawer and Escape closes it.

Property changes refresh the preview after 500 ms of inactivity. A bad value keeps the last successful preview and shows an error. Local drafts retain unfinished input after one second, including values such as an incomplete minus sign. Saving a template or exporting requires valid geometry and property values. Content that fails rendering, such as an undersized barcode, may be saved for later correction but cannot produce a successful export.

## Fonts, images, and barcode data

**Document font** uses the template's imported TrueType/OpenType font, or the original monospace fallback when none is imported. Import a font containing the required glyphs for Chinese and other scripts; choosing a different built-in face does not add missing glyphs. Images are flattened over white before embedding. Threshold values below 128 produce less black ink; above 128 produce more. Contain preserves the whole image, Cover crops to fill, and Stretch changes its aspect ratio.

Module width `0` chooses the largest whole-dot module that fits. A positive width is rounded to dots at the current 8 or 12 dots/mm resolution. Too-small or too-large modules are rejected, rather than distorting the symbol. Quiet zone `0` uses the format's minimum; an explicit value must meet that minimum. QR uses four modules on each side, as described in the [DENSO WAVE guide](https://www.qrcode.com/en/howto/code.html). The inspector displays the minimum for each format.

EAN-13, EAN-8, and UPC-A accept the full number or the body without its final check digit. Incorrect supplied check digits are rejected. ITF requires an even number of digits and does not insert padding. Code 39 uses its uppercase/digit/punctuation alphabet. Code 128 and PDF417 currently accept ASCII; use QR or Data Matrix for UTF-8 content. The designer does not automatically add GS1 application identifiers, FNC1, or HIBC prefixes. Physical print quality and scanner settings still affect readability; automated decoder checks do not replace printed-label acceptance.

## Saving and compatibility

Templates embed images and the document font and remain self-contained. The expanded designer reads version 1 and version 2 templates; saving writes version 2. Earlier app versions cannot read version 2, so keep a copy of a version-1 file if you need to reopen it with an older app. Existing version-1 defaults preserve their previous rendered appearance.

PDF combines the document pages into one file; PNG saves one image per page. Exports capture the content and options when submitted and continue in the background after a tab or window closes. Designs do not export printer-language commands, and opening a ZPL/EPL/TSPL/CPCL/DPL file keeps it as a source document rather than reconstructing design objects.

## Rendering quality (unreleased)

In **Export → Rendering**, choose **Raster** for the existing output or **Vector** to retain native text, shape outlines and barcode modules in PDF. Imported bitmap images remain images. Both modes preserve object order, backgrounds, clipping, quarter-turn rotations and physical label dimensions.

**Antialias** retains gray edge pixels in previews and PNG output, and in raster PDF. It is off by default. Vector PDF stores geometry; its edge smoothing is determined by the viewer. Vector PNG offers **1×–4×** resolution without changing the label's physical size. It is a bitmap rendered from the scene, not a vector file.

Rendering choices belong to the tab's local session and export snapshot, rather than the portable template format. They survive tab/session restoration and do not mark the source file as modified. Existing templates and settings keep the original black-and-white raster defaults.
