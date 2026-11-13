# Designer compatibility baseline

`design-v1.labelize.json` uses the original template schema, with no style block.
`design-v1.png` was generated at 8 dots/mm with the native macOS daemon built from
`0800fe3` (the renderer predating the expanded designer), on 2026-11-10. It pins
the appearance of text, Code 128, QR quiet zones, and an outline rectangle.

`version_one_templates_keep_their_previous_release_pixels` compares decoded
pixels before and after upgrading the template to version 2. Do not regenerate
this baseline just to accommodate a change in legacy rendering.

## BarTender format detection

`bartender-sample.btw` is an unchanged copy of
[`Sample_Doc1.btw` from Seagull's BarTender Cloud API examples](https://github.com/Seagull-Scientific/bartender-cloud-api/blob/9c0fbe4291dee0c5fdda13de38d69ba71b0a8975/Sample_Doc1.btw).
Its [MIT license](bartender-sample.LICENSE) is included alongside the fixture.
