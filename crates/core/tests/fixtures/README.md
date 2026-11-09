# Designer compatibility baseline

`design-v1.labelize.json` uses the original template schema, with no style block.
`design-v1.png` was generated at 8 dots/mm with the native macOS daemon built from
`0800fe3` (the renderer predating the expanded designer), on 2026-11-10. It pins
the appearance of text, Code 128, QR quiet zones, and an outline rectangle.

`version_one_templates_keep_their_previous_release_pixels` compares decoded
pixels before and after upgrading the template to version 2. Do not regenerate
this baseline just to accommodate a change in legacy rendering.
