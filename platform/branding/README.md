# Labelize Studio brand assets

The Studio mark is a folded white label shaped like an **L**, with three barcode cuts and a cyan corner on a cobalt-blue tile. It represents the label workspace, designer, and print output in one mark.

## Source and generated assets

`labelize-studio.png` is the transparent, high-resolution master. It was generated with the built-in OpenAI image generation tool on 2026-11-10 and refined to keep the outer contour clean. Preserve its alpha channel when exporting.

Run from the repository root on macOS with ImageMagick installed:

```sh
bash scripts/build-icons.sh
```

The script generates:

- `platform/icon.png` — 256 px, embedded in GPUI and displayed in the README.
- `platform/linux/labelize-printer.png` — 512 px, installed alongside the 256 px PNG in the Linux `hicolor` theme.
- `platform/windows/labelize-printer.ico` — 16, 24, 32, 48, 64, 128, and 256 px representations for Windows and NSIS.
- `platform/macos/LabelizePrinter.icns` — standard and Retina representations through 1024 px for macOS.

The `labelize-printer` asset names and application identifiers are retained for upgrade compatibility. Commit the master and generated files together. Rebuild the desktop executable after changing the PNG because GPUI embeds the logo at compile time.

## Generation prompt

```text
Use case: logo-brand
Asset type: production desktop app icon and project logo for Labelize Studio, a clean technical workspace for label design, printer-language preview, and PDF/PNG export.
Primary request: create one original, finished app icon. A bold folded white label ribbon forms a distinctive geometric L, with three short dark-blue barcode cuts in its lower foot; a small icy-cyan folded corner gives the label a precise crafted detail. Integrate these into one coherent emblem, not separate pictograms.
Style/medium: refined vector-like geometry, clean edges, restrained dimensional shading, premium modern productivity app identity. Confident, calm, technological, fresh.
Color palette: saturated electric blue to slightly deeper cobalt tile, bright white emblem, restrained cyan fold accent. High contrast at small sizes.
Composition: square 1024 by 1024 canvas, one centered macOS-style rounded-square tile occupying about 90% of canvas width, evenly inset on every side. Emblem centered and large with generous internal padding and a sturdy silhouette. True transparent pixels outside the tile; preserve softly antialiased edges.
Constraints: icon only, no wordmark, no typography, no spelling, no extra symbols, no sparkle, no cursor, no pencil, no literal printer hardware, no multiple variants, no presentation board, no device mockup, no scene, no checkerboard background, no watermark. Keep the silhouette readable at 16-32 pixels.
```

## Refinement prompt

```text
Use case: precise-object-edit
Asset type: final production app icon with transparency.
Edit the supplied Labelize Studio icon. Preserve the exact white folded L label emblem, cyan corner, three dark blue barcode slots, blue gradient, and overall composition.
Change only the exterior contour and alpha: make the blue rounded-square tile a mathematically smooth, precise rounded rectangle with uniformly clean antialiased edges and equal transparent padding on all four sides. Remove ALL stray blue specks, flecks, brush-like ragged extensions and noise outside its contour. Outside the single smooth rounded square every pixel is fully transparent. NO drop shadow or glow outside the tile. The outside corners and border must be flawless on both white and black backgrounds.
Return one square app-icon asset, centered, with the tile occupying 88% of the square canvas. No text, no caption, no mockup or grid, no new design elements. Keep the original interior design unchanged.
```
