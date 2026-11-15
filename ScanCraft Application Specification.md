# ScanCraft Application Specification

**Version 2.0, draft for review** · 15 November 2026 · ArtCraft Team

ScanCraft is a macOS desktop app that turns photos of paper documents into clean, searchable PDF files. The user drops in photos or PDFs. ScanCraft finds each page's edges, flattens the page, even when it is bent like the page of an open book, evens out lighting and contrast, reads the text, and writes one PDF in which every page is an image with an invisible, selectable text layer lying exactly over the ink.

Version 2.0 replaces the three-page v1 specification (`ScanCraft Application Specification.pdf`). It keeps v1's product and three-screen flow, and adds the exact algorithms, where each one comes from, its parameters, and the tests that decide when it is done.

## Contents

1. [Product](#1-product)
2. [What changed from v1](#2-what-changed-from-v1)
3. [User flow](#3-user-flow)
4. [Screens and interface](#4-screens-and-interface)
5. [Technology decisions](#5-technology-decisions)
6. [Architecture](#6-architecture)
7. [Conventions](#7-conventions)
8. [Algorithms](#8-algorithms)
9. [Performance budgets and limits](#9-performance-budgets-and-limits)
10. [Testing and evaluation](#10-testing-and-evaluation)
11. [Milestones](#11-milestones)
12. [Open decisions and risks](#12-open-decisions-and-risks)
13. [References](#13-references)

---

## 1. Product

### 1.1 What ScanCraft does

- Imports JPEG, PNG, HEIC/HEIF, TIFF, WebP and PDF files by drag and drop or through the Open panel.
- Finds the outline of the page in each photo, curved sides included, and lets the user correct it with a magnifier.
- Flattens the page by mapping its four corners to a rectangle with the paper's true proportions, so an A4 sheet comes out A4.
- Flattens curved pages too: book pages, curled receipts, folded letters and sheets held in the hand come out flat, with straight lines of text and the paper's true proportions. A photo of an open book can become its two pages, in reading order.
- Cleans the page with one of four looks: Original, Black & White, Grayscale, Color Document.
- Reads the text on the Mac and writes one PDF with an invisible text layer, so the PDF can be searched and its text selected and copied.

### 1.2 Principles

1. **On-device and private.** No network access, accounts or telemetry. The photos' EXIF data (location, camera model) never reaches the PDF. Logs hold only fixed step names, durations and counts, never file names or content.
2. **Every page is a recipe.** A page stores its source photo and the settings applied to it: corners and curved sides, flattening, rotation, look and strength. Pixels are rendered from the recipe, so any step can be changed later or undone, and each page is compressed only once.
3. **Never crash, never lose work.** The Craft never-crash standard applies (§6.8), and the session saves itself after every change.
4. **Deterministic.** The same photo and recipe give byte-identical output on any machine and with any number of threads. The only exception is Apple Vision, whose results can change between macOS releases.
5. **Rust first.** The app is Rust end to end. Apple frameworks are used only where they are clearly better and Rust has no equivalent: finding the page and recognising text.

### 1.3 Not in this release

- Camera capture. Importing from an iPhone with Continuity Camera can come later.
- Flattening pages bent in two directions at once, such as crumpled sheets or a corner curled diagonally. ScanCraft flattens pages that bend in one direction, folds included (A27); §12.1 lists the option for the rest.
- Editing PDF content beyond page order, rotation and look.
- Windows and Linux apps. The core crates stay portable, and the command-line tool builds on Linux for CI.

---

## 2. What changed from v1

| Topic | v1 | v2 | Why |
|---|---|---|---|
| UI toolkit | Not specified; AppKit panels named | egui/eframe in Rust, the toolkit of the other Craft apps | Shared menus, dialogs and test tooling; one language (§5.1) |
| Decoding | macOS ImageIO | Pure-Rust `craft-image-codecs`, with HEIC through `craft-heif`; ImageIO only if HEIC decoding misses its time budget | Portable, testable on Linux, no `unsafe` (§5.5) |
| Detection input | Grayscale (`cv2.cvtColor`) | Colour: each pixel's gradient comes from whichever of R, G or B changes most | Grayscale can erase the edge between white paper and a pale-coloured desk (A5) |
| Finding the page | Blur, Canny, Suzuki, then Douglas–Peucker on the dominant contour | Three sources of candidate outlines (Apple's page detector, contours, Hough lines), one shared score, then sub-pixel edge refinement at full resolution | Classical detection alone leaves about half of real photos needing manual correction; a neural detector plus refinement, about 15% (A8) |
| Page proportions | Implied by the side lengths of the outline | Recovered from the corners and the camera's focal length (Zhang–He), then snapped to A4, Letter, Legal, ID-1 or business card when within 3% | Side lengths misjudge proportions by 6–21% at 25–30° of tilt (A11) |
| Perspective matrix | 3 × 3 matrix, method not stated | Heckbert's closed-form mapping from a square to a quadrilateral, in f64 | Exact; no linear solver (A12) |
| Warping | `cv2.warpPerspective` | Inverse mapping with Catmull–Rom bicubic sampling, then Lanczos-3 resampling to the target DPI | Sharp text without aliasing (A13, A14) |
| Shadow removal | Median blur with a huge kernel, then division | Even Light: estimate the paper's brightness only from bare-paper pixels (text, photos, logos and fingers masked out), fill the gaps, divide | A large median blur takes any big dark or coloured area for shadow and bleaches it (A16) |
| Black & White | Adaptive Gaussian thresholding | Midpoint between the local lightest and darkest values, with a contrast gate | Measured F-measure 0.989, against 0.959 for Sauvola and 0.935 for a global threshold, and it keeps pale pencil (A18) |
| Color Document | CLAHE | Tone curve plus colour-preserving saturation, after Even Light | On a mostly white page CLAHE turns the paper grey and triples its grain (A17) |
| OCR | Vision, run at export | Vision (`VNRecognizeTextRequest`) in the background as soon as a page is added; `ocrs` as the portable fallback | Export is fast; the command-line tool runs on Linux (A20) |
| PDF | PDFKit / CoreGraphics | Pure-Rust writer on `pdf-writer`, with the text-layer code ported from PdfCraft: JPEG for colour and grey, JBIG2 for black and white, invisible text in rendering mode 3 | Deterministic, testable anywhere, smaller files (A21–A23) |
| Curved pages | Not handled | The page is modelled as a sheet of paper bent in one direction, fitted to its outline and text lines in the photo with the camera's focal length, then unrolled. A book spread becomes two pages. | A four-corner warp leaves book pages, curled receipts and folded letters with curved lines and squeezed text. In a numerical prototype on synthetic bent pages, the fit placed the content 10–40 times more accurately than the corners alone (A25–A30) |
| New | — | Page tray (reorder, delete, re-edit), rotation and automatic upright, edge handles, bend handles, strength slider, export options, undo, session restore, command-line tool | Needed for real batches |

---

## 3. User flow

### 3.1 Steps

1. **Import (Screen 1).** The user drops photos or PDFs, or picks them in the Open panel. Files are decoded in the background in order. Page detection runs ahead for every imported photo, so the crop screen opens with the corners already placed.
2. **Crop (Screen 2), once per photo.** The detected outline appears over the photo, following the page's edges where they curve. The user drags corners or sides if needed, using the loupe for precision, then chooses **Correct Perspective**. With **Book Spreads** on, a photo of an open book is split at the spine into its two pages.
3. **Enhance (Screen 3), once per page.** The flattened page appears. The user picks a look and, if needed, adjusts strength or rotation, then chooses **Add Page**. The next photo opens on Screen 2; after the last one, Screen 1 returns.
4. **Export.** **Export to PDF** is available on every screen once the tray holds a page. ScanCraft asks where to save, finishes any text recognition still running, and writes the file.

### 3.2 Screen states

| State | Shows | Leads to |
|---|---|---|
| Import | Screen 1, plus the tray if pages exist | Crop, for the first photo that is ready |
| Crop (photo *i*) | Screen 2 | Enhance (*i*) through Correct Perspective; Crop (*j*) through the batch arrows; Import when no photos are left |
| Enhance (page *i*) | Screen 3 | Crop (next photo) or Import through Add Page; Crop (*i*) through Back to Crop |
| Export | Save sheet, then a progress sheet | The state it was opened from |

Reopening a page from the tray enters Enhance for that page, and Back to Crop works from there too. A photo split into the two pages of a book spread passes through Enhance once for each page, in reading order.

---

## 4. Screens and interface

### 4.1 Window and visual design

- Default window 1200 × 800 pt, minimum 860 × 600 pt. Size and position are restored at launch.
- Light and dark appearance follow the system.
- Colours come from the Craft theme tokens. ScanCraft adds three brand colours sampled from the app icon (`scancraft.jpeg`): Copper `#A84128` for primary buttons, Light Copper `#D87840` for hover and pressed states, and Scan Cyan `#02FDFE` for the page outline, handles and focus ring.
- The canvas background is a fixed dark neutral (`#1C1C1E`) in both appearances, so page edges stay easy to see.
- Text uses the system UI font through the system font provider in the Craft fork of egui.

### 4.2 Screen 1: Import Hub

| Element | Behaviour |
|---|---|
| Drop zone | As v1: a dashed rounded rectangle (2 pt dashes, 12 pt corner radius) filling 80% of the content area, labelled "Drop photos or PDFs here", with the supported formats underneath. While files hover over the window, its border turns Scan Cyan and its fill takes an 8% cyan tint. A drop starts the import and opens Screen 2 as soon as the first photo is ready (target: 600 ms for a 12-megapixel HEIC). |
| Browse Files… | As v1: the macOS Open panel (`NSOpenPanel`, through `craft-dialogs` `FileDialog::open_many()`) with multiple selection and a filter for jpg, jpeg, png, heic, heif, tif, tiff, webp and pdf. Shortcut ⌘O. |
| Formats | JPEG, PNG, HEIC/HEIF and PDF (v1), plus TIFF (every page) and WebP, which the decoder supports at no extra cost. A PDF adds one item per page and a TIFF one item per page. Any other file is skipped with a message naming it. |
| Folders | A dropped folder adds its supported files, not those in subfolders. |
| Order | Items keep the order of the drop. Within a folder or a multi-file selection they are sorted by name in natural order ("IMG_9" before "IMG_10"). |
| Restore banner (new) | If the last session was never exported: "Continue your last scan (7 pages)?" with Continue and Start New. |
| Page tray (new) | Shown when the session already has pages (§4.5). |

### 4.3 Screen 2: Boundary Detection and Cropping

| Element | Behaviour |
|---|---|
| Image canvas | As v1: the photo scaled to fit the area above the tray, with 32 pt padding. The canvas shows the display copy, whose long side is at most 3072 px. |
| Page outline | Drawn as a 2 pt Scan Cyan line over a 1 pt black halo at 50% opacity. Outside the outline, the photo is dimmed by a 45% black scrim. Straight sides are drawn straight. A side that A25 found curved follows its curve, and once the page is flattened the outline is the fitted one (A28). |
| Crop nodes (4 corners) | As v1. 16 pt white circles with a 2 pt cyan ring; the hit area has a 22 pt radius. Dragging places the corner with sub-pixel precision in photo pixels. A corner may go up to 15% of the photo's width or height outside the photo, for pages cut off by the frame; the part of the page outside the photo becomes white. Moving any node turns Auto-Detect off. |
| Edge handles (new) | Pill-shaped handles (6 × 22 pt) at the middle of each side. Dragging one moves that side along its normal while its two corners slide along the neighbouring sides, so those sides keep their angles. If a neighbouring side is nearly parallel, both corners move by the drag instead. On a curved side, the bend handles take the edge handle's place, and Shift-dragging the middle one moves the whole side. |
| Bend handles (new) | Each curved side has three bend handles, 10 pt white dots with a 2 pt cyan ring, at a quarter, half and three quarters of its length. Dragging one bends the side through it. The side becomes the smooth curve (a centripetal Catmull–Rom spline) through its two corners and three handles, A27 takes that curve as the side's evidence, Flatten turns On and Auto-Detect turns off. Double-clicking a straight side gives it bend handles; Straighten Side in the context menu removes them. |
| Flatten (new) | A three-way control beside Auto-Detect: Auto (default), On and Off. Auto flattens the page when the checks of A29 pass; On always uses the fitted sheet (A27); Off uses the four corners only. The tooltip gives the outcome and its reason, such as "Flattened: text lines 64% straighter" or "Left flat: no bend found". Unavailable, with a tooltip saying why, for Use Full Image and for pages imported from a PDF. |
| Book Spreads (new) | A toggle. When on, each photo of an open book becomes the spread's two pages (A30). It stays set for the session and becomes the next session's default. With Book Spreads off, a photo that looks like a spread shows a small "Two pages? Split" button in the canvas corner. |
| Spine (new) | For a split spread, a dashed Scan Cyan line along the spine, with a round handle at each end. Dragging an end moves that end of the spine, and both pages' outlines follow. Each page has its own corners and sides on the outer edges. "Join Pages" in the context menu undoes the split. |
| Invalid shape | If the quadrilateral of the four corners stops being convex, any corner angle leaves the range 20°–160°, or a bent side crosses another side, the outline turns red and Correct Perspective is disabled with a tooltip saying why. A curved side may dip inward, as a book page's top edge does near the spine. The move itself is never blocked. |
| Magnifier loupe | As v1: appears while a node, edge handle, bend handle or spine end is pressed or dragged, and for 1 s after a keyboard nudge. A 140 pt circle showing the photo at 3× the canvas zoom, sampled from the full-resolution photo rather than the display copy. Its centre sits 110 pt above the pointer, moves to 110 pt below when the pointer is within 130 pt of the window's top, and stays inside the window. It shows a crosshair (1 pt lines with a 6 pt gap) on the exact corner position, plus the two sides that meet at that corner. For a bend handle or a spine end, it shows the curve or the spine through it. |
| Auto-Detect toggle | As v1, defined as a toggle: on means the corners and curved sides are the detected ones. Moving a node or bending a side turns it off. Turning it on restores the detected outline from the cache at once. Its tooltip names the source: Apple, Contours, Lines or Not found. |
| Use Full Image (new) | Puts the corners on the photo's corners, for photos that are already cropped. Default for items imported from a PDF. |
| Correct Perspective | As v1. Primary button; Return key. Computes the page proportions (A11), flattens the page (A12–A14, or A28 for a curved page) and opens Screen 3. The first frame comes from the display copy within 100 ms; the full-resolution page replaces it when ready. |
| Discard Photo (new) | Removes the photo from the batch. Undoable. |
| Batch position (new) | "Photo 2 of 7", with previous and next arrows. |
| Not-found hint | When detection finds nothing, the corners start 3% inside the photo's edges and the canvas shows: "Couldn't find the page edges. Drag the corners onto the page." |
| Keyboard | Tab and Shift-Tab select corners in the order top-left, top-right, bottom-right, bottom-left, then the bend handles side by side, then the spine's two ends. Arrow keys move the selected corner or handle by 1 photo pixel; with Shift by 10 px; with Option by 0.25 px. Escape clears the selection. |

### 4.4 Screen 3: Filtering and Export

| Element | Behaviour |
|---|---|
| Page canvas | As v1: the flattened page, fit to the area above the tray. Pressing and holding on the page shows the Original look for comparison. |
| Filter strip | As v1: Original, Black & White, Grayscale, Color Document. Each is a 112 × 150 pt thumbnail rendered from a 512 px preview, with its name below. A click applies the look (A17, A18). Left and right arrows move the selection; keys 1–4 pick one directly. The first page starts on Color Document; each later page starts on the look the previous page used. |
| Strength (new) | A 0–100 slider for Color Document (default 60), Grayscale (60) and Black & White (55); hidden for Original. The canvas updates live from a 1280 px preview, and the full-resolution page renders after 300 ms without changes. |
| Rotate (new) | Rotate Left (⌘L) and Rotate Right (⌘R), in quarter turns. When automatic upright (A15) turned the page, a small note says so, and Undo reverts it. |
| Back to Crop (new) | Returns to Screen 2 with this page's corners. Shortcut ⌘[. |
| Flattening note (new) | When Auto flattened the page, a small note under it says "Flattened curved page", and its Undo sets Flatten to Off. A page from a spread says "Left page of a spread" or "Right page of a spread". |
| Add Page | As v1: adds the page to the tray, starts its text recognition in the background, then opens the next pending photo on Screen 2, or Screen 1 when none is left. After the first page of a spread it opens the spread's other page on Screen 3 instead, starting on the same look. The label reads "Add Page & Next" while photos or pages remain. Shortcut ⌘Return. |
| Export to PDF | As v1, with two differences: the current page is added first, and text recognition has usually finished in the background already. Shortcut ⌘E. |

### 4.5 Page tray (new)

A 112 pt bar along the bottom of every screen once the session has a page:

- Thumbnails (64 × 88 pt) with page numbers, in export order.
- Drag to reorder. Click to reopen the page on Screen 3. Delete removes the selected page (undoable). The context menu offers Edit Crop, Rotate Left, Rotate Right and Delete.
- At the right end: the page count and the Export to PDF button, with a chevron that opens the export options.

### 4.6 Export options and Settings

Export options, in a popover from the Export chevron, saved between sessions:

| Option | Values |
|---|---|
| Page size | Auto (default: the detected or snapped size from A11) · A4 · US Letter · US Legal · A5. A fixed size fits each page image inside it, centred on white, without stretching. |
| Quality | Small: 200 DPI, JPEG quality 70 · Balanced (default): 300 DPI, quality 82 · Best: 300 DPI, quality 92 with full-resolution colour, and Black & White at 400 DPI |
| Searchable text | On (default) · Off |
| Text language | Automatic (default), or any language Vision reports as supported |

Settings window (⌘,):

| Setting | Default |
|---|---|
| Default paper | From the system region: US Letter in the regions the Unicode CLDR data lists for Letter (BZ, CA, CL, CO, CR, GT, MX, NI, PA, PH, PR, SV, US, VE), A4 everywhere else |
| Snap to standard paper sizes | On |
| Rotate pages upright automatically | On |
| Flatten curved pages automatically | On: new pages start with Flatten on Auto. Off: they start on Off. |
| Book page order | Left to right. Right to left when the system's first preferred language is written right to left, such as Arabic, Hebrew, Persian or Urdu. |
| Language correction in text recognition | On. Turn off for forms full of codes and numbers, which language correction can "fix" into words. |
| Keep paper colour | Off. When on, Even Light keeps tinted paper tinted instead of making it white. |
| Session storage | `~/Library/Application Support/ScanCraft/Sessions`; keeps the 10 most recent sessions |

The default file name is "Scan 2026-11-14 at 21.04.07.pdf" (the date and time of export), saved in the last folder used, initially `~/Documents`.

### 4.7 Menus and shortcuts

A native macOS menu bar through `craft-menu`. Each item runs a command by id (§6.3).

| Menu | Items |
|---|---|
| ScanCraft | About ScanCraft · Settings… ⌘, · Hide ⌘H · Quit ⌘Q |
| File | New Session ⇧⌘N · Import… ⌘O · Export to PDF… ⌘E · Export Options… ⌥⌘E |
| Edit | Undo ⌘Z · Redo ⇧⌘Z · Delete Page ⌫ |
| Page | Auto-Detect ⌘D · Use Full Image ⇧⌘D · Flatten ▸ Auto, On, Off · Straighten Side · Book Spreads ⌥⌘B · Correct Perspective ↩ · Back to Crop ⌘[ · Rotate Left ⌘L · Rotate Right ⌘R · Look ▸ Original ⌘1, Black & White ⌘2, Grayscale ⌘3, Color Document ⌘4 · Add Page ⌘↩ · Previous Photo ⌘← · Next Photo ⌘→ |
| View | Show Page Tray ⌥⌘T |
| Window, Help | Standard items; Help opens a page bundled with the app |

### 4.8 Feedback and errors

| Situation | What the user sees |
|---|---|
| A file can't be read or isn't supported | The file is skipped; a message names it and gives the reason |
| A photo is over 100 megapixels | Skipped, with the reason |
| No page found | Corners start 3% inside the photo's edges, with the hint on the canvas |
| Flatten is On but the page has no text lines or curved edges to fit, or the fit fails | The page is flattened by its corners alone, and Screen 3 says: "Couldn't flatten the curve: no lines of text or curved edges to follow." |
| Book Spreads is on but no spine is found | The photo stays one page, with the note "No spine found" and a Split at Centre button |
| Page resolution under 150 DPI | A note on Screen 3 that small text may be hard to read; nothing is blocked |
| Text recognition fails on a page | Export continues; the completion message lists the pages that aren't searchable |
| Imported photos haven't been added | "3 photos haven't been added. Export the 5 added pages?" with Export and Cancel |
| Write error (disk full, permissions) | An alert with the system's reason. Nothing partial is left at the destination (A23). |
| Quitting during an export | One question: Cancel Export or Keep Exporting. A system restart, shutdown or log-out never waits: the session is already saved, the temporary file is deleted, and the app exits (`craft-quit`). |

### 4.9 Accessibility and localization

- Every control has an AccessKit role and label, so VoiceOver can read and operate it. Corners are adjustable elements ("Top-left corner, 412, 380"), and so are bend handles ("Top side, bend handle 2 of 3") and the spine's ends.
- The whole flow works from the keyboard (§4.3, §4.7).
- Reduce Motion removes transitions. Increase Contrast thickens outlines to 3 pt.
- Strings live in `craft-i18n` resource files. Dates, numbers and the default paper size follow the system locale.

---

## 5. Technology decisions

### 5.1 UI toolkit: egui

**Decision:** egui with eframe, at the version and fork WordCraft pins (egui 0.37, `storytold/egui`).

What the interface needs, and where it comes from:

- File drag and drop, with hover feedback: egui's `hovered_files` and `dropped_files`, fed by winit.
- Native Open and Save panels: `craft-dialogs`. Open uses `NSOpenPanel`; Save is a sheet on the window.
- Native menu bar: `craft-menu`, built on muda.
- Quit, restart and shutdown handling: `craft-quit`.
- The canvas, outline, handles and loupe: GPU textures (`TextureHandle`), a textured mesh for the round loupe, and `Painter` for lines and handles.
- Accessibility: egui's AccessKit integration.
- UI snapshot tests: `egui_kittest`.

| Option | Verdict | Reason |
|---|---|---|
| egui + eframe | **Chosen** | Pure Rust. Already used by WordCraft, PdfCraft and BissCraft, so `craft-ui`, `craft-menu`, `craft-dialogs`, `craft-quit` and the snapshot-test tooling work unchanged. The trade-off: the app looks like the Craft family, not stock AppKit. |
| Tauri | Rejected | Needs a JavaScript or TypeScript front end, against the Craft rule "Rust only, no handwritten JS/TS". Every 12-megapixel preview and loupe crop would cross the webview bridge, and none of the Craft UI crates apply. |
| Iced | Rejected | Capable, but it would rebuild what the team already has for egui (menus, dialogs, docking, test harness) with no gain the user can see. |
| SwiftUI + UniFFI | Rejected for v2 | Gives the most native look, but adds Swift and Xcode to the build, splits the app across two languages, ties the interface to macOS and loses the shared tooling. Worth revisiting only if a stock AppKit look becomes a requirement. |

### 5.2 Image processing: in-house, plus a few imageproc functions

**Decision:** the algorithms live in ScanCraft's own crates (§6.1), written from the papers cited in §8. A few well-defined pieces come from `imageproc` 0.27 because they match exactly: contour tracing, convex hull, polygon simplification, Hough lines and connected components.

| Option | Verdict | Reason |
|---|---|---|
| `opencv` crate | Rejected | Needs C++ OpenCV and libclang at build time and tens of megabytes of libraries to sign and notarize. Its C++ calls sit outside Rust's safety checks, and it can't build for WebAssembly. Each algorithm ScanCraft needs is short. |
| `kornia-rs` | Rejected | Good resizing, colour conversion and warping, but its API is still pre-1.0, and most of ScanCraft's specific steps (contours, line voting, document cleanup) would still be ours. |
| `cv` crate | Rejected | Built for multi-view geometry and camera pose. The only overlap is RANSAC, which here is a 40-line seeded loop. |
| `imageproc` 0.27 (October 2026) | **Used for seven functions** | `find_contours` (Suzuki–Abe), `convex_hull`, `approximate_polygon_dp`, `arc_length` and `contour_area`, plus `hough::detect_lines` and `region_labelling::connected_components`. MIT licensed, pure Rust. Some of these panic on empty input or ε ≤ 0, so they are called through wrappers that check first (§6.8). Its Canny accepts only grayscale and always adds a fixed σ = 1.4 blur, and its adaptive threshold is mean-only, so neither fits. |
| In-house | **Chosen for the rest** | Colour Canny, scoring, refinement, proportions, warping, resampling, Even Light, looks and Black & White, and for curved pages the side tracing, the text-line tracker, the sheet fit with its least-squares solver, and the spread split. Each is specified in §8 and is a few hundred lines at most, deterministic, and free of panics and `unsafe`. |

### 5.3 PDF writing: `pdf-writer`, with PdfCraft's text layer ported

**Decision (15 November 2026):** `scancraft-io` writes the PDF with `pdf-writer` 0.15. It ports from PdfCraft (Apache-2.0, with its notice kept in `NOTICE` and in the ported files) the four pieces ScanCraft needs:
- the invisible text layer, `pdfcraft-ocr`'s `text_layer`;
- the word fitting, `pdfcraft-fonts`' `TextPlan`;
- the Type3 font registrar with ToUnicode mappings, from `pdfcraft-edit`'s `add_type3_fonts`;
- the image pages, from `pdfcraft-create`'s `from_images`: JPEG passes through untouched as `/DCTDecode`, and two-level images stay 1-bit.

The port records the PdfCraft commit it was taken from. PdfCraft fixes to these pieces are copied across by hand, and its tests for them come along with the code.

| Option | Verdict | Reason |
|---|---|---|
| `pdf-writer` 0.15, with PdfCraft's text layer ported | **Chosen** | Keeps PdfCraft's tested behaviour: invisible text in rendering mode 3, wrapped in `q … Q`, each word stretched to its box, Unicode mappings for any script. `pdf-writer` (MIT OR Apache-2.0) is small and already in the tree through krilla. ScanCraft's craft-libs revision stays its own. |
| PdfCraft crates pinned at a git revision | Rejected | The four pieces pull in about ten other PdfCraft crates, including XFA forms, annotations and the ocrs engine, which `pdfcraft-ocr` doesn't make optional. PdfCraft pins all of craft-libs at one revision, so ScanCraft would have to use exactly that revision, or Cargo builds two incompatible copies of `craft-pdf` and `craft-image-codecs`. PdfCraft had over 100 commits in the 30 days before this decision. |
| The pieces moved into craft-libs first | Later, not now | One shared copy is the right end state, but the move is a refactor in PdfCraft's repository of unknown size. Once ScanCraft's ported module stands alone, it can move to craft-libs and PdfCraft can share it (§12.1). |
| `krilla` (the Craft apps' usual PDF writer, 0.8.2 through `craft-pdf`) | Rejected here | Has no invisible text rendering mode (VectorCraft #3159). Drawing text at opacity 0 instead is the wrong primitive: it paints a zero-alpha glyph and breaks PDF/A-1, which bans transparency. |
| `printpdf` | Rejected | Could write invisible text, but its higher-level model gets in the way of the exact word fitting being ported. |
| `pdf` (pdf-rs) | Rejected | Built mainly for reading; writing is not its strength. |
| PDFKit / CoreGraphics | Rejected | macOS only, can change between OS releases, and can't run in Linux CI. |

### 5.4 Text recognition: Apple Vision, with ocrs as fallback

| Option | Verdict | Reason |
|---|---|---|
| Apple Vision `VNRecognizeTextRequest` | **Chosen on macOS** | Runs on the Mac, is fast, reads many scripts and gives word boxes. `objc2-vision` 0.3.2 marks nearly every call ScanCraft needs as safe (§6.7). |
| `ocrs` 0.13 | Fallback | Pure Rust, used by PdfCraft; ScanCraft calls it directly, in the command-line tool only. Reads Latin script only; its models are CC-BY-SA-4.0 files kept outside the binary. Used by the command-line tool on Linux and in CI. |
| Tesseract | Not bundled | C++. PdfCraft can call an installed copy; ScanCraft doesn't need it. |

### 5.5 Decoding and colour: craft-libs

| Option | Verdict | Reason |
|---|---|---|
| `craft-image-codecs` with `craft-heif` and `craft-cms` | **Chosen** | Pure Rust with no `unsafe`. Applies EXIF orientation and HEIF rotation, decodes iPhone grid HEICs, keeps ICC profiles, and checks image sizes before allocating. |
| ImageIO | Fallback | Only if pure-Rust HEIC decoding misses its 400 ms budget (§9), decided by measurement in M1. |

### 5.6 Flattening curved pages: a fitted model of the sheet

**Decision:** model the page as a sheet of paper bent in one direction, fit that model to the page's outline and text lines in the photo, and unroll it (A25–A30). No network and no new dependency.

| Option | Verdict | Reason |
|---|---|---|
| Fitted sheet model, in-house | **Chosen** | Paper bends without stretching, so a page bent in one direction is fully described by its profile and its position in front of the camera: about 20 numbers. In a numerical prototype on synthetic book pages, curls, sags and letters folded in three, the fit placed the page's content within 0.05–0.11% of the page's diagonal (median), against 1.1–2.4% for the corners alone (A27). The user's corrections become more evidence for the same fit. Deterministic, explainable, and within a 40 ms budget per page (§9). Its limit is one bend direction per page. |
| UVDoc network (Verhoeven, Magne and Sorkine-Hornung 2023) through rten | Deferred (§12.1) | The model pdfcraft-android chose after comparing published methods with released weights: about 8 million parameters, 15.8 MB in FP16, and it also handles pages bent two ways. Against it: the repository is MIT, but the trained weights carry no licence of their own; on pdfcraft-android's ten reference photos it moved flat pages by up to 2% of their diagonal and needed a guard (#302); it was trained on single sheets, not spreads; it can't take the user's corrections; and it takes 0.39–0.55 s per page on a desktop. |
| Other networks (DewarpNet, DocTr, DocGeoNet) | Rejected | Behind UVDoc in its published comparison: on the UVDoc benchmark, MS-SSIM (a measure of image similarity, 1 being identical) of 0.589, 0.697 and 0.706 against UVDoc's 0.785. |
| Apple Vision or VisionKit | Not available | No public API flattens a bent page. macOS 26's `RecognizeDocumentsRequest` reads a document's structure but doesn't correct its shape. |
| Port of page_dewarp (Zucker 2016, MIT) | Model adopted, code not | Python with OpenCV and SciPy. Its general-purpose optimiser took 5–25 s per page on a 2012 laptop, and its page width ignores the profile's length, which squeezes text where the page slopes steeply. |

### 5.7 Dependencies

| Crate | Version or pin | Licence | Used for |
|---|---|---|---|
| `egui`, `eframe`, `egui_kittest` | 0.37, storytold fork at WordCraft's revision | MIT OR Apache-2.0 | Interface, UI tests |
| `craft-image-codecs` (feature `heif`), `craft-heif`, `craft-cms`, `craft-metadata` | craft-libs, git revision | MIT OR Apache-2.0; `heic-rs` MIT OR Apache-2.0 | A1, A2 |
| `craft-jpeg`, `craft-stream-codecs` | craft-libs | MIT OR Apache-2.0 | A22 |
| `craft-pdf` (features `read`, `raster`) | craft-libs | MIT OR Apache-2.0 | A24 |
| `craft-dialogs`, `craft-menu`, `craft-quit`, `craft-ui`, `craft-keymap`, `craft-settings`, `craft-i18n`, `craft-atomic` | craft-libs | MIT OR Apache-2.0 | App shell, settings, crash-safe writes |
| `pdf-writer` | 0.15 | MIT OR Apache-2.0 | A21, A23 |
| Code ported from PdfCraft (`text_layer`, `TextPlan`, the Type3 registrar, `from_images`) | storytold/pdfcraft at a recorded commit | Apache-2.0, notice kept | A21, A23 |
| `ocrs` 0.13.1, `rten` 0.26 | `scancraft-cli` only, not the Mac app | MIT OR Apache-2.0; models CC-BY-SA-4.0 | OCR fallback (command-line tool, Linux) |
| `imageproc` | 0.27 | MIT | A6, A7, A19 |
| `image` | 0.25 | MIT OR Apache-2.0 | Pixel buffers for `imageproc` calls |
| `objc2`, `objc2-foundation`, `objc2-vision` | 0.6, 0.3, 0.3.2; macOS only | Confirm SPDX in M0 | A8, A15, A20 |
| `rayon` | 1 | MIT OR Apache-2.0 | Parallel row bands |
| `unicode-segmentation` | 1 | MIT OR Apache-2.0 | Splitting recognised lines into words |
| `serde`, `serde_json`, `sha2`, `thiserror` | 1, 1, 0.11, 2 | MIT OR Apache-2.0 | Session file, hashes, errors |

Curved pages (A25–A30) add no dependency: the spline, the solver and its Cholesky factorisation are in-house, in `scancraft-geom`.

---

## 6. Architecture

### 6.1 Crates and layers

| Layer | Crate | Contents |
|---|---|---|
| L0 | `scancraft-core` | Shared types only: geometry primitives, pixel buffers and the band iterator, the page recipe types of §6.2, the command list, the traits of A8 and A15, the `EngineApi` the interface uses, errors, `ALGO_VERSION`. Fixed first, so the other crates can be built in parallel (see the build plan) |
| L0 | `scancraft-geom` | Points, lines, quadrilaterals, canonical corner order, homography (A12), proportions and paper sizes (A11), the camera, the bent-sheet model and its unrolling (A27, A28), the cubic spline, and the Levenberg–Marquardt solver with its small dense matrix routines |
| L1 | `scancraft-raster` | Pixel buffers (RGB8, Gray8, 1-bit, f32 planes), A3, A4, the A13 warp (through A12's homography or A28's unrolled sheet), A14, the A19 helpers, A18's ink rule (steps 1–5, shared with A26), colour (sRGB, Oklab), `ALGO_VERSION` |
| L2 | `scancraft-detect` | A5–A7, A9, A10, the `PageDetector` trait that A8 implements, and for curved pages A25, A26 and A30 |
| L2 | `scancraft-enhance` | A15 upright (through an `UprightVoter` trait), A16–A18 |
| L2 | `scancraft-io` | Import (A1, A2, A24), encoding (A22), PDF export (A21, A23) |
| L3 | `scancraft-engine` | Sessions and page recipes, commands, undo, jobs, caches, persistence, the flatten decision (A29), and OCR scheduling over the `Recognizer` trait in `scancraft-core` |
| L3 | `scancraft-apple` (macOS) | Vision page detector, recogniser and upright voter; optional ImageIO decoder |
| L4 | `scancraft-ui-egui` | Screens, canvas, loupe, tray, menus, settings |
| Apps | `scancraft`, `scancraft-cli` | The Mac app; the command-line tool |
| Tool | `scancraft-synth` | The synthetic scene generator and the evaluation metrics (§10), with its own camera and sheet code so that it checks the pipeline independently; used by `scancraft-cli synth` and `eval`, never shipped in the app |

`cargo xtask layers` enforces the layering as in WordCraft: a crate depends only on lower layers, and nothing below L4 depends on egui, eframe or winit. Only the two apps depend on `scancraft-apple`, and they plug its implementations into the engine's traits, so every other crate builds and tests on Linux.

### 6.2 The page recipe

```rust
pub struct Page {
    pub id: PageId,
    pub photo: PhotoId,                   // the batch item it was cut from; a spread gives two pages
    pub source: SourceId,                 // photo or PDF page, copied into the session folder
    pub corners: Quad,                    // TL, TR, BR, BL in photo pixels (§7)
    pub corners_from: CornerOrigin,       // Detected { by, score } | NotFound | Manual | FullImage
    pub sides: [SideShape; 4],            // top, right, bottom, left: Straight | Traced { samples, bow, curved } | Bent { handles } | Spine (A25, A30)
    pub spread: Option<SpreadHalf>,       // Left or Right page of a book spread, with the spine line (A30)
    pub flatten: FlattenMode,             // Auto | On | Off
    pub sheet: Option<SheetFit>,          // the fitted sheet and the decision's record, when the page is flattened (A27, A29)
    pub proportions: Option<Proportions>, // ratio, how it was found, snapped paper, native size (A11, or A28 for a flattened page)
    pub page_size_mm: Option<(f64, f64)>,
    pub quarter_turns: u8,                // 0..=3, clockwise
    pub turns_from: TurnOrigin,           // Auto | Manual
    pub look: Look,                       // Original | BlackWhite | Grayscale | ColorDocument
    pub strength: u8,                     // 0..=100
    pub text: Option<RecognizedText>,     // engine, engine version, lines of words
}
```

The definitions live in `scancraft-core` (`recipe.rs`), with the session file that stores them.

Rendering is a pure function of the recipe, the source pixels and `ALGO_VERSION`. Cache keys are hashes of exactly those inputs (§6.5).

### 6.3 Commands

Every action the user can take is a command with an id, parameters and an undo record, as in WordCraft. Buttons, menus, shortcuts, the command-line tool and tests all run commands by id, and a command run by a program never opens a dialog.

`import.add {paths}` · `import.discard {item}` · `crop.set_corner {page, corner, x, y}` · `crop.move_side {page, side, offset}` · `crop.bend_side {page, side, handles}` · `crop.straighten_side {page, side}` · `crop.set_flatten {page, mode}` · `crop.book {on}` · `crop.split {photo, on}` · `crop.set_spine {photo, top, bottom}` · `crop.auto` · `crop.full_image` · `crop.apply` · `page.set_look {look}` · `page.set_strength {value}` · `page.rotate {quarter_turns}` · `page.add` · `page.delete` · `page.move {from, to}` · `page.edit {page}` · `export.pdf {path, options}` · `session.new` · `session.restore` · `edit.undo` · `edit.redo`

The list, with each command's parameters, is `scancraft_core::command::Command`.

Undo keeps one stack per session, up to 200 steps. A whole drag counts as one step.

### 6.4 Jobs and threading

- The UI thread only draws and dispatches commands. Any pixel work longer than about 2 ms goes to the job runner.
- The job runner is one scheduler thread with a priority queue, plus a rayon pool for pixel loops. Priorities, highest first: previews for the visible page; detection for upcoming photos; full-resolution renders; text recognition. Every job carries a cancellation token, and changing a recipe cancels the jobs that became stale.
- Vision calls run on one dedicated thread, each inside an autorelease pool.
- Results reach the UI through a channel, followed by `ctx.request_repaint()`.
- Pixel loops split work into fixed 64-row bands, so results don't depend on the number of threads (§7).

### 6.5 Rendering paths and caches

| Path | Size | Used for |
|---|---|---|
| Work image | Long side 1024 px | Detection (A5–A9) |
| Flatten analysis copies | Long side 1600 px (1000 px for A30) | A26 and A29: the page straightened by its corners, and once more by the fitted sheet; A30: the spread straightened by its outline |
| Display copy | Long side up to 3072 px | Screen 2 canvas; uploaded to the GPU once per photo |
| Loupe crop | A square from the full-resolution photo, 1.5× the area the loupe shows, at most 1024 px | The loupe. Cut again when the node moves more than a quarter of the crop's side from its centre. Drawn as a textured circle of 64 segments. |
| Look preview | Long side 1280 px | Screen 3 while editing. Must match the final page within a mean difference of 6 grey levels. |
| Thumbnails | 512 px for the filter strip, 256 px for the tray | |
| Final page | Output DPI | Encoded once (A22); those exact bytes go into the PDF |

Cached results are files in the session folder, each named by a SHA-256 hash of its inputs: source hash, corners rounded to 1/64 px, side shapes, spread half and spine, Flatten setting, the fitted sheet's parameters, quarter turns, look, strength, DPI and `ALGO_VERSION`. At most two decoded full-resolution photos stay in memory, least recently used first out.

### 6.6 Sessions and persistence

- Each session is a folder holding `session.json` (recipes, page order, options), `sources/` (copies of the imported files) and `cache/`. On APFS, `std::fs::copy` makes clones, which take no extra space until one copy changes.
- The session is saved through `craft-atomic` after every command, at most once per second.
- At launch, a session that was never exported brings up the restore banner (§4.2).
- Cleanup keeps the 10 most recent sessions. A session that was never exported is removed only after the user confirms.

### 6.7 Platform crate and the one `unsafe` exception

- `scancraft-apple` (macOS only) holds every call into Apple frameworks: the Vision page detector (A8), text recognition (A20), upright voting (A15) and, if needed, ImageIO decoding.
- It uses `objc2-vision` 0.3.2, which marks the calls ScanCraft needs as safe: `VNImageRequestHandler` `initWithData:options:` and `performRequests:error:`; `VNRecognizeTextRequest` `new`, `setRecognitionLevel:`, `setUsesLanguageCorrection:`, `setRecognitionLanguages:`, `setAutomaticallyDetectsLanguage:`, `setMinimumTextHeight:` and `results`; `VNRecognizedTextObservation` `topCandidates:`; `VNRecognizedText` `string` and `confidence`.
- One method has no binding: objc2-vision's translation config skips `VNRecognizedText` `boundingBoxForRange:error:`, which gives the box of each word. It is sent with `objc2::msg_send!` at a single call site. Any accessor the bindings generate as `unsafe`, such as the corners of `VNRectangleObservation` if so generated, is wrapped the same way.
- This crate is the repository's only named exception to `unsafe_code = "forbid"`, recorded in AGENTS.md. It sets `unsafe_code = "deny"` and allows `unsafe` only on those call sites, each with a `// SAFETY:` comment, and its tests run on macOS CI.

### 6.8 Never crash

The Craft never-crash standard applies, as in WordCraft:

- No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!` or `unimplemented!` outside tests. Crate roots deny the matching Clippy lints.
- Errors are `Result` values with each crate's error type.
- Every number derived from input is treated as hostile: checked arithmetic, `get()` instead of indexing, capped allocations, bounded loops.
- Calls into `imageproc` go through wrappers that check its preconditions first (non-empty input, at least 3 points, ε > 0, image sides under 65 535 px), because those functions panic otherwise.
- The job runner turns an escaped panic into an error for that page. This is a last-resort guard, not a licence.
- Every crash fix lands with a regression test, and a fuzz test runs every command with junk parameters.

### 6.9 Command-line tool

`scancraft-cli` runs the same engine without a window, for automation, evaluation and Linux CI (where OCR uses ocrs):

```text
scancraft-cli detect IMG_0412.HEIC --json
scancraft-cli page IMG_0412.HEIC --look color --strength 60 --flatten auto --out page.png
scancraft-cli flatten IMG_0412.HEIC --json
scancraft-cli pdf *.HEIC --look bw --ocr auto --out scans.pdf
scancraft-cli pdf book/*.HEIC --book --order rtl --out book.pdf
scancraft-cli synth --count 500 --seed 7 --out synth/
scancraft-cli synth --curved --count 300 --seed 7 --out synth-curved/
scancraft-cli eval --set smartdoc2015 --root ~/datasets/smartdoc --report eval.json
scancraft-cli eval --set uvdoc --root ~/datasets/UVDoc_benchmark --report flatten.json
```

### 6.10 Packaging

- Bundle identifier `ai.storyteller.scancraft`, following WordCraft's `ai.storyteller.wordcraft`.
- Minimum macOS 13 Ventura, for Vision's automatic language detection and third text-recognition revision (decided, §12.1).
- Universal binary (arm64 and x86_64), Developer ID signing with the hardened runtime, notarized DMG. No App Sandbox in v2; the session copies its sources, so a later sandboxed build needs no file bookmarks.
- Document types (viewer role) for JPEG, PNG, HEIC, HEIF, TIFF, WebP and PDF, so "Open With" and dropping files on the Dock icon import them.
- App icon from `scancraft.jpeg`, which first needs a transparent master (§12.1).

---

## 7. Conventions

1. **Coordinates.** x points right and y down. Pixel (*i*, *j*) covers [*i*, *i*+1) × [*j*, *j*+1), and its centre is (*i* + ½, *j* + ½). Scaling an image by *k* scales coordinates by exactly *k*.
2. **Corners.** Stored in pixels of the oriented, full-resolution photo, in the order TL, TR, BR, BL (clockwise on screen). Canonical order for any four points: sort by `atan2(y − cy, x − cx)` around their centroid, ascending (clockwise on screen, because y points down), then rotate the list to start at the point with the smallest x + y.
3. **Colour.** sRGB with 8 bits per channel at rest. Processing uses f32 in [0, 1] on sRGB-encoded values unless "linear" is stated. Luma, the single brightness channel, is Y = 0.299 R + 0.587 G + 0.114 B on encoded values; these are the weights A16's parameters were measured with.
4. **Sizes.** DPI means page pixels per inch at the page's physical size. Physical sizes are in millimetres and PDF sizes in points (1 pt = 1/72 in). A parameter written "*n* px @300 DPI" becomes *n* · D/300 at D DPI, and window sizes round to the nearest odd integer.
5. **Image borders.** Separable filters extend an image by mirroring without repeating the edge pixel (… c b | a b c d | c b …). Minimum and maximum filters repeat the edge pixel. Warping fills points outside the photo with white.
6. **Determinism.** Work is split into fixed 64-row bands. Sums and histograms are built per band and combined in band order. Random sampling uses xorshift64* seeded from a stable hash of the page id and the step. `ALGO_VERSION`, an integer in `scancraft-raster`, changes whenever any algorithm's output changes. It is part of every cache key and is written into the PDF's Producer field.
7. **Percentiles.** P*q* of a set of values comes from a 1024-bin histogram over the values' range: the lower edge of the first bin where the cumulative count reaches *q*%.
8. **Camera.** A pinhole camera with its principal point at the photo's centre and a focal length of f · L pixels, where L is the photo's long side and f comes from A11 or is fitted by A27. Camera coordinates have x to the right, y down and z away from the camera, so a point (X, Y, Z) appears at (W/2 + f·L·X/Z, H/2 + f·L·Y/Z).

---

## 8. Algorithms

### 8.0 Pipeline and sources

```text
photo ─ A1 decode, orient ─ A2 to sRGB ─┬─ A3 work image (1024 px) ─ A5 colour edges ─┬─ A6 contours ──┐
                                        │                                             └─ A7 lines ─────┤
                                        │                A8 Apple page detector (macOS) ───────────────┤
                                        │                                         A9 score and select ─┴─ A10 refine ─ corners
                                        └─ A3 display copy (3072 px) ─ Screen 2

corners ─ A25 curved sides ─ A30 split a book spread (Book Spreads on) ─ A11 focal length, proportions ─ A26 text lines
        ─ A27 sheet fits ─ A29 flatten? ─┬─ no:  A12 homography ─────────────────┬─ A13 warp ─ A14 resample ─ A15 upright
                                        └─ yes: A28 unrolled sheet, proportions ─┘
        ─ A16 Even Light ─ A17 / A18 look ─ A22 encode ─ page in the tray

page ─ Grayscale look (A17) ─ A20 text recognition ─ A21 invisible text layer ─┐
pages in tray order ───────────────────────────────────────────────────────────┴─ A23 PDF

PDF files ─ A24 render pages ─ treated as photos
```

| Step | Source | Reference |
|---|---|---|
| A1 Decode and orient | `craft-image-codecs` (feature `heif` → `craft-heif` → `heic-rs`), `craft-metadata` | EXIF; ISO/IEC 23008-12 (HEIF) |
| A2 To sRGB | `craft-cms` | ICC profiles |
| A3 Area downscale | In-house | Same result as OpenCV `INTER_AREA` |
| A4 Gaussian blur | In-house | Kovesi 2010 for large σ |
| A5 Colour edges | In-house | Canny 1986 |
| A6 Contour candidates | `imageproc` 0.27 | Suzuki and Abe 1985; Graham 1972; Ramer 1972; Douglas and Peucker 1973 |
| A7 Line candidates | `imageproc` 0.27 | Duda and Hart 1972 |
| A8 Apple page detector | `objc2-vision`: `VNDetectDocumentSegmentationRequest` | Apple Vision, macOS 12+ |
| A9 Score and select | In-house | — |
| A10 Edge refinement | In-house; design from pdfcraft-android #291 | Fischler and Bolles 1981 |
| A11 Proportions | In-house; design from pdfcraft-android #293 | Zhang and He 2007 |
| A12 Homography | In-house | Heckbert 1989 |
| A13 Warp | In-house | Keys 1981 |
| A14 Resample | In-house | Duchon 1979; Turkowski 1990 |
| A15 Upright | Vision (fast); fallback ported from `pdfcraft-ocr`'s `detect_orientation` | — |
| A16 Even Light | In-house; design from pdfcraft-android #298 | Knutsson and Westin 1993; van Herk 1992; Gil and Werman 1993 |
| A17 Looks | In-house; tone formula from pdfcraft-android #298 | — |
| A18 Black & White | In-house; method from pdfcraft-android #300 | Bernsen 1986; Bayer 1973 |
| A19 Shared helpers | In-house; `imageproc` connected components | van Herk 1992; Paeth 1990; Ottosson 2020 |
| A20 Text recognition | `objc2-vision`: `VNRecognizeTextRequest`; fallback `ocrs` in the command-line tool | Unicode UAX #29 |
| A21 Invisible text layer | Ported from PdfCraft (`text_layer`, `TextPlan`, the Type3 registrar), on `pdf-writer` | ISO 32000-2 §9.3.6 |
| A22 Image encoding | `craft-jpeg`, `craft-stream-codecs` | ITU-T T.81 (JPEG); ITU-T T.88 (JBIG2) |
| A23 PDF assembly | `pdf-writer`; image pages ported from `pdfcraft-create`; `craft-atomic` | ISO 32000-2 |
| A24 PDF import | `craft-pdf` (`read`, `raster`) | — |
| A25 Curved sides | In-house | Montanari 1971 |
| A26 Text lines | Port of pdfcraft-android's strip tracker (#288, #303) | — |
| A27 Sheet fit | In-house; model from Zucker 2016 and pdfcraft-android #303, fitted in the photo instead | Liang, DeMenthon and Doermann 2008; Kim, Koo and Cho 2015; Levenberg 1944; Marquardt 1963; Nielsen 1999; Triggs et al. 2000; Zhang 2000; Higham 1986 |
| A28 Unrolling | In-house | — |
| A29 When to flatten | In-house; final check from pdfcraft-android #302 | — |
| A30 Book spreads | In-house; gutter search from pdfcraft-android #303 | — |

pdfcraft-android (github.com/MohamedRef/pdfcraft-android) is a community fork of PdfCraft, licensed MIT OR Apache-2.0, that built and measured a pure-Rust document scanner in 2026. ScanCraft adopts several of its designs together with the numbers it measured. Its code can be ported with attribution if it passes ScanCraft's tests. Its work on curved pages (#302, #303) also showed two things to avoid: a flattening network that moved flat pages, and a text-line fit that had lost the camera's distance (A27).

### A1. Decode and orient

**Source:** `craft-image-codecs::decode` from craft-libs, with the `heif` feature, which decodes HEIC through `craft-heif` and the pure-Rust `heic-rs` decoder.

1. Identify the format from the file's bytes, not its name.
2. Decode with limits of 32 768 px per side and 100 megapixels. The decoder checks the header's dimensions before allocating.
3. The decoder applies the EXIF Orientation tag (JPEG, PNG, WebP, TIFF) and HEIF's rotation, mirror and crop boxes (`irot`, `imir`, `clap`), so pixels come back upright.
4. Composite any transparency over white: c′ = a·c + (1 − a).
5. Read `FocalLengthIn35mmFilm` (EXIF tag 0xA405) and `DigitalZoomRatio` (0xA404) with `craft-metadata` for A11. For HEIC, `craft-heif::items::read` supplies the EXIF block. Nothing else from EXIF is kept.
6. Copy the file into the session's `sources/` folder and record its SHA-256.

`craft-heif` refuses 4:2:2 and 4:4:4 HEIF (the "HIF" files from some Canon and Sony cameras). The message says this HEIF variant isn't supported, unless the ImageIO fallback exists (§9).

**Tests:** the eight EXIF orientations give identical upright pixels; an iPhone 12-megapixel grid HEIC and a 48-megapixel HEIC decode; empty, truncated, 1 × 100 000 px and 70 000 × 70 000 px-header files return errors without large allocations.

### A2. Convert to sRGB

**Source:** `craft-cms`, pure-Rust ICC colour management.

1. If the file embeds an ICC profile: `Profile::parse`, then `Transform::new(&profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, …)` with black-point compensation on, taken from the shared `transform::cached` store, then `convert_u8` or `convert_u16`.
2. HEIF that declares its colour as `nclx` primaries without an ICC profile: use the matching built-in profile (Display P3, Rec. 2020 or sRGB).
3. No colour information: treat the image as sRGB.
4. 10-bit and 16-bit sources convert at full depth, then round to 8 bits.

**Why sRGB:** page images go into the PDF as RGB that viewers read as sRGB, paper documents rarely hold colours outside sRGB, and one working space keeps every threshold in this section meaningful.

**Tests:** a Display P3 photo of a grey card stays neutral (channels within 1 level of each other); results match `craft-cms`'s single-colour `eval` within 1 level.

### A3. Area downscale

**Source:** in-house (`scancraft-raster`).

Each output pixel is the average of the source area it covers, with partial weights for source pixels cut by its boundary. For a scale *k* = L_out / L_in < 1, output column *i* covers source x ∈ [*i*/*k*, (*i* + 1)/*k*). Weights are computed once per column and applied horizontally, then vertically, with f32 sums.

Used for the work image (1024 px), the display copy (3072 px), the Even Light analysis copy (1000 px), previews (1280 px) and thumbnails.

**Tests:** an integer factor gives exact block means; a constant image stays constant; the image's mean is preserved within 1e−5.

### A4. Gaussian blur

**Source:** in-house.

- σ ≤ 8 px: separable convolution with weights g(t) = exp(−t² / 2σ²) for |t| ≤ ⌈3σ⌉, normalised to sum to 1.
- σ > 8 px: three box filters in a row (Kovesi 2010), which cost the same for any σ. With n = 3: w_ideal = √(12σ²/n + 1); w_l = the largest odd integer ≤ w_ideal; w_u = w_l + 2; m = round((12σ² − n·w_l² − 4n·w_l − 3n) / (−4w_l − 4)). Apply m boxes of width w_l, then n − m boxes of width w_u.
- σ must lie in [0.3, 256]. Any other value is an error, never a panic.

**Tests:** the response to a single bright pixel sums to 1, and its variance is σ² within 2% (direct) or 5% (boxes).

### A5. Colour edges

**Source:** in-house. Canny (1986), with the rule OpenCV uses for multi-channel input: each pixel takes its gradient from the channel that changes most.

**Idea.** Canny's detector marks thin lines where brightness changes fastest, then keeps weaker edge pixels only where they connect to strong ones. This step, called hysteresis, keeps long faint edges without picking up isolated noise.

Input: the work image (RGB, long side 1024 px).

1. Blur each channel with A4, σ = 1.5 px. This softens wood grain and fabric so they don't read as edges.
2. Per channel, Sobel gradients scaled so that a ramp rising by 1 per pixel gives 1:
   ```text
   gx(x,y) = [ I(x+1,y−1) + 2·I(x+1,y) + I(x+1,y+1) − I(x−1,y−1) − 2·I(x−1,y) − I(x−1,y+1) ] / 8
   gy(x,y) = [ I(x−1,y+1) + 2·I(x,y+1) + I(x+1,y+1) − I(x−1,y−1) − 2·I(x,y−1) − I(x+1,y−1) ] / 8
   ```
3. At each pixel, keep the channel with the largest gx² + gy². Magnitude G = √(gx² + gy²); direction θ = atan2(gy, gx).
4. Thin the edges (non-maximum suppression): round θ, taken modulo 180°, to 0°, 45°, 90° or 135°. Keep a pixel only if its G is greater than its neighbour's in the gradient direction and at least its neighbour's in the opposite direction; the unequal comparison stops edges from coming out two pixels wide. Clear the outermost pixel of the image.
5. Set thresholds from this image's statistics, over the remaining pixels with G > 0.005:

   | Map | High threshold T_h | Low threshold T_l |
   |---|---|---|
   | Strict | P80, clamped to [0.040, 0.200] | 0.4 · T_h |
   | Sensitive | P60, clamped to [0.015, 0.100] | 0.4 · T_h |

6. Hysteresis: start from remaining pixels with G ≥ T_h and grow through 8-connected remaining pixels with G ≥ T_l, using an explicit stack rather than recursion.

Output: the binary edge maps E_strict and E_sens, plus G and θ for A9 and A10.

**Why colour.** White paper (0.95, 0.95, 0.95) on a pale yellow desk (0.97, 0.95, 0.80) differs from the desk by only 0.011 in luma but by 0.15 in blue. A grayscale edge detector misses that page.

**Why two maps.** The strict map gives clean outlines on ordinary photos; the sensitive map still finds faint page edges. Both feed candidates, and A9's score decides between them.

**Tests:** step edges with contrast 0.1 at every 15° are found along ≥ 95% of their length, within 1 px; the pale yellow desk above is found; edges are one pixel wide.

### A6. Contour candidates

**Source:** `imageproc` 0.27: `contours::find_contours` (Suzuki–Abe border following), `geometry::convex_hull` (Graham scan), `geometry::approximate_polygon_dp` (Ramer–Douglas–Peucker), `geometry::arc_length` and `geometry::contour_area`, called through wrappers that check their preconditions (§6.8).

**Idea.** Border following traces the outline of every connected group of edge pixels. The convex hull wraps a rubber band around an outline. Douglas–Peucker drops points from an outline until only its main corners remain.

For each edge map E in {E_strict, E_sens}:

1. Close one-pixel gaps: dilate E with a 3 × 3 square.
2. Trace outer borders with `find_contours::<i32>`, keeping contours of type `BorderType::Outer` with at least 16 points.
3. Keep the 24 contours with the largest hull areas.
4. For each contour, take its hull H. Skip it if `contour_area(H)` is under 5% of the image's area. Let P = `arc_length(H, true)`.
   1. Simplify H with ε = f · P for f = 0.010, 0.015, 0.020, 0.030, 0.040, 0.060, 0.080 in turn. The first result with exactly four points is a candidate. Stop early if fewer than four points remain.
   2. Four longest sides: simplify H with ε = 0.01 · P. If at least five points remain, take its four longest edges in outline order, extend each into a line, and intersect neighbouring lines. The result is a candidate when all four intersections exist, with no two neighbouring lines within 3° of parallel, and lie in the allowed area. This recovers a corner hidden under a thumb or cut off by the frame, which otherwise appears as a fifth, bevelled vertex.

Allowed area for corners, for every candidate: x ∈ [−0.15 w, 1.15 w] and y ∈ [−0.15 h, 1.15 h] on a w × h image.

**Why the hull.** If one side of the page is faint and its edge breaks, the outline becomes a U shape. Its hull still closes the page.

### A7. Line candidates

**Source:** `imageproc` 0.27 `hough::detect_lines`, the standard Hough transform.

**Idea.** Every edge pixel votes for all the straight lines that could pass through it. Lines that real edges support collect many votes, even when the edges are broken into pieces.

1. Run `detect_lines(&E_sens, LineDetectionOptions { vote_threshold: max(25, round(0.08 · min(w, h))), suppression_radius: 6 })`.
2. Rank the lines by support, counted as the E_sens pixels within 1 px of each line, because `detect_lines` doesn't return vote counts. Keep the top 16.
3. Add the image's four borders as lines marked "border".
4. Split the lines into two direction groups: 2-means clustering on the doubled-angle unit vectors (cos 2φ, sin 2φ), seeded with the best line's direction and its perpendicular, for 10 iterations. Doubling the angle makes 0° and 179° neighbours.
5. Take every pair of lines within one group whose directions differ by at most 35° and that lie at least 0.15 · min(w, h) apart where they cross the line through the image centre, and every such pair from the other group. The four intersections form a quadrilateral. Put it in canonical order and keep it if it is convex with all corners in the allowed area.
6. Keep at most 1024 candidates, preferring the largest total line support.

Hough angles come in whole degrees. At 1024 px a half-degree error moves a corner by up to about 4.5 px, which A10's ±16 px search absorbs.

### A8. Apple page detector (macOS)

**Source:** Apple Vision `VNDetectDocumentSegmentationRequest` (macOS 12 and later), a neural network that finds a document and returns its four corners, called through `objc2-vision` in `scancraft-apple`.

1. Encode the work image as PNG, pass it to `VNImageRequestHandler` through `initWithData:options:`, and perform the request with a 2 s time limit.
2. Take the first result, a `VNRectangleObservation`. Its corners are normalised to [0, 1] with the origin at the bottom left. Convert them with x = u · w and y = (1 − v) · h, and put them in canonical order.
3. No result, or an error, means no candidate from this source.

The observation's `confidence` is not used: for Vision's related rectangle request it has been observed to be 1.0 for every result, and nobody has shown that document segmentation behaves better. A9 judges the result instead.

**Why.** Neural detectors find pages that edge-based methods miss, such as low-contrast edges and busy backgrounds. Genius Scan reports that 51% of photos need no manual correction with classical detection, 75% with a neural detector and 85% with a neural detector plus edge refinement (as cited in pdfcraft-android #286). Vision gives ScanCraft such a detector without shipping a model.

### A9. Score and select

**Source:** in-house.

1. **Validity.** Reject a candidate that isn't convex and simple; has a corner angle outside [35°, 145°]; has a pair of opposite sides whose lengths differ by more than 3×; covers less than 5% of the image; or has a corner outside the allowed area.
2. **Edge support, per side.** Sample the middle 90% of the side every 1 px, with n the outward normal. A sample counts as in the image if it lies at least 2 px from every border. It is supported if, within ±τ px along n (τ = 3 before refinement, 2 after), some pixel is in E_sens and has a gradient direction within 25° of n, in either sense. support_k = supported samples / in-image samples.
   A side is a border side when fewer than 30% of its samples are in the image, or when at least 70% of them lie within 3 px of the image's border. Its support_k is fixed at 0.35, which lets a page larger than the frame still win.
3. **Contrast, per side that isn't a border side.** At each in-image sample s, Δ is the Euclidean distance in Oklab (A19) between the blurred work image 4 px inside (s − 4n) and 4 px outside (s + 4n); contrast_k is the median Δ. C = the mean over those sides of min(1, contrast_k / 0.08). C = 0.5 when every side is a border side.
4. **Area.** A = the candidate's area inside the image ÷ the image's area.
5. **Score.** S = E² · C · √A, where E is the mean support_k. If any side that isn't a border side has support under 0.2, halve S.
6. **Selection.**
   1. Score every candidate from A6, A7 and A8 with τ = 3.
   2. Refine (A10) and trace (A25) the three best classical candidates that differ from each other (pairwise overlap IoU < 0.9) and the Apple candidate, then score them again with τ = 2. A side that A25 marks curved is scored along its traced path instead of its straight line, so a bent page isn't marked down for bending.
   3. If the Apple candidate is valid, use it, unless the best classical candidate scores S_c > 1.5 · S_apple and S_c ≥ 0.12.
   4. Otherwise, use the best classical candidate if S_c ≥ 0.12.
   5. Otherwise, report "not found" and start the corners 3% inside the photo's edges.

The constants in step 6 are starting values. Milestone M1 tunes them on the reference set (§10.2) for the highest no-touch rate and records the measured numbers next to them in the code.

**Why this score.** E² rewards outlines that real edges support all the way round. C prefers boundaries where the colour changes, as between paper and desk, over lines printed on the page, which have paper on both sides. √A prefers the whole page over a table or text block inside it, without letting size alone decide.

**Tests:** on synthetic scenes (§10.1) the true page is chosen in ≥ 98% of clean cases; with a ruled frame 12 px inside the page, the outer edge wins; a desk with no page gives "not found"; a page larger than the frame is found using border sides.

### A10. Sub-pixel edge refinement

**Source:** in-house; design and measurements adapted from pdfcraft-android #291.

**Idea.** Each side of the page is a long, straight step in brightness. Refinement looks across that step at many places along the side, finds the step's exact position on each look to a fraction of a pixel, and fits a straight line through the positions that agree. The corners are where neighbouring lines cross.

Input: a quadrilateral in work-image pixels, and the luma of the full-resolution photo.

Pyramid: level 0 is the work image (long side 1024 px). Each finer level doubles the size, made with A3 from the full-resolution luma, up to the last level, which is the full-resolution photo itself.

For each level from coarse to fine, and each side:

1. **Search width.** Half-width r = 16 px at level 0 and 6 px at finer levels.
2. **Scan lines.** N = clamp(round(side length / 4), 32, 400) lines, evenly spaced, skipping 8% of the side at each end. Each runs along the outward normal n from −r to +r.
3. **Profile.** Luma sampled bilinearly every 0.5 px, smoothed with a Gaussian (σ = 1 px), and differentiated with central differences.
4. **Polarity.** Whether the page is lighter or darker than what lies outside it, from the means of the first and last 1.5 px of all the profiles. Only steps in that direction count.
5. **Candidates per scan line.** Up to four local extremes of the derivative with the side's polarity and a magnitude of at least 25% of the median, over scan lines, of each line's strongest extreme. Each position is refined with a parabola through the three samples around it and weighted by its magnitude.
6. **Choose the line.** Draw 200 random hypotheses (xorshift64*, seeded per page, side and level), each the line through two candidates from two different scan lines. A hypothesis's support is the number of scan lines with a candidate within 1.0 px of it. Among hypotheses with at least 70% of the best support, take the outermost, the one with the largest mean offset along n. Fit it to its inlier candidates by weighted total least squares (the principal axis of the weighted points).
   **Why outermost:** a ruled frame or table border just inside the page is as strong as the paper's edge on any single scan line, but the paper's edge is the outermost line that most scan lines agree on.
7. **Accept or keep.** Accept the fit if its inliers come from at least 40% of the scan lines and it turns the side by less than 6°. Otherwise keep the side as it was and mark it unrefined.
8. **Corners.** Intersect neighbouring sides, then multiply by 2 for the next level.

Final checks at full resolution: no corner moved more than 2 · 16 · (full-resolution long side / 1024) px; the quadrilateral is still convex; its area is between 0.8× and 1.25× the input's. If any check fails, the input comes back unchanged. Corners may end up outside the photo by up to 15% of its width or height.

Refinement runs once, after detection. It never moves corners the user placed.

**Tests** (adapted from #291; synthetic 12-megapixel scenes): clean page on a dark desk, starting up to 25 px off: mean error ≤ 0.3 px, P95 ≤ 0.8 px. Blur σ = 1.5 px with noise σ = 6 levels: mean ≤ 0.6 px, P95 ≤ 1.5 px. A ruled frame 12 px inside the page: the outer edge is chosen. A finger over 60% of one side: that side is unrefined or within 2 px, the others within 0.8 px. A checked tablecloth: every corner within 2 px. A corner 5% outside the frame: within 3 px. An exact starting quadrilateral moves by less than 0.2 px. Refining twice changes corners by less than 0.3 px. One thread and eight threads give identical results.

### A11. True proportions and page size

**Source:** in-house; Zhang and He (2007), as specified and measured in pdfcraft-android #293.

**Idea.** A rectangle photographed at an angle looks like an irregular quadrilateral, and its side lengths in the photo no longer give its shape. Its four corners still hold enough information to recover the camera's focal length, and from that the rectangle's real width-to-height ratio.

Input: refined corners in photo pixels, photo size W × H, and the focal length from EXIF if there is one.

1. **Normalise.** L = max(W, H). m_i = ((x_i − W/2)/L, (y_i − H/2)/L, 1), with m1 = TL, m2 = TR, m3 = BL, m4 = BR. The order is a Z, not a walk round the page: m3 is bottom-left and m4 is bottom-right.
2. **Vanishing directions.**
   ```text
   k2 = ((m1 × m4) · m3) / ((m2 × m4) · m3)
   k3 = ((m1 × m4) · m2) / ((m3 × m4) · m2)
   n2 = k2 · m2 − m1
   n3 = k3 · m3 − m1
   ```
3. **Focal length f**, in units of L:
   - Known: when EXIF gives F35, the 35 mm-equivalent focal length, and the photo has the shape of a full camera frame (4:3, 3:2 or 16:9 within 1%, either way round), f_k = F35 · √(W² + H²) / (43.27 · L). If DigitalZoomRatio Z > 1, also compute f_k · Z and use whichever is closer to f_e when f_e is valid; otherwise use f_k.
   - Estimated: when |n2z| > 1e−4 and |n3z| > 1e−4, f_e² = −(n2x·n3x + n2y·n3y) / (n2z·n3z). Valid if f_e² > 0 and 0.35 ≤ f_e ≤ 4.0.
   - Use f_k if available, else f_e if valid, else 0.75, a typical phone main camera (26 mm equivalent on a 4:3 frame).
4. **Ratio** r = width ÷ height of the page as it lies in the photo:
   ```text
   r = sqrt( (n2x² + n2y² + f²·n2z²) / (n3x² + n3y² + f²·n3z²) )
   ```
   If r isn't finite or falls outside [1/20, 20], use side lengths instead: r = (|TR − TL| + |BR − BL|) / (|BL − TL| + |BR − TR|). Record which path gave r: KnownFocal, EstimatedFocal, TypicalFocal or SideLengths.
5. **Snap** (on by default). Let ρ = max(r, 1/r). If ρ is within 3% of a standard ratio, replace it with that ratio, keeping the orientation, and remember the paper. The closest match wins.

   | Paper | Long ÷ short | Size (mm) |
   |---|---|---|
   | ISO A series (A4 unless the user picks another) | 1.4142 | 210 × 297 |
   | US Letter | 1.2941 | 215.9 × 279.4 |
   | US Legal | 1.6471 | 215.9 × 355.6 |
   | ID-1 card | 1.5858 | 85.60 × 53.98 |
   | US business card | 1.7500 | 88.9 × 50.8 |

6. **Physical size.** A snapped page takes its paper's size. Any other page fits inside the default paper (§4.6) in the matching orientation, as large as possible with ratio r.
7. **Native pixel size.** Ŵ = max(|TR − TL|, |BR − BL|) and Ĥ = max(|BL − TL|, |BR − TR|). If Ŵ/Ĥ ≥ r, use W_n = Ŵ and H_n = Ŵ/r; otherwise H_n = Ĥ and W_n = r · Ĥ. Round, then cap at 40 megapixels by scaling both down equally. This keeps the full sampling of the denser direction and invents none in the other.

For a page that A29 decides to flatten, A28 supplies the ratio of step 4 and the side lengths of step 7, measured along the bent sheet; snapping and the physical size work as above.

**Why.** Side lengths get the ratio wrong as soon as the camera tilts: by 6.4% at 25° and 15° of tilt, and by 21% for a 4 × 6 print at 30° and 10° (L-K-M/Kirsch #23). The Zhang–He estimate had a median error of 0.12% (P95 0.51%) on 2000 synthetic views in pdfcraft-android #293.

**Tests** (from #293): synthetic cameras with ratios 1.4142, 1.2941, 1.0 and 2.5, focal lengths 0.6–1.2, tilts 0–50°, any roll, and 0.5 px of corner noise on 4000 × 3000 photos. Tilt ≥ 10° with an estimated focal length: median error ≤ 1%, P95 ≤ 3%. Known focal length: median ≤ 0.5%, P95 ≤ 1.5%. Degenerate inputs (a pair of parallel sides, an exact rectangle, a negative f²) give a finite ratio, the right path label and no panic. Scaling the photo leaves r unchanged, and turning it by 90° gives 1/r. A ratio of 1.40 snaps to A4 and 1.36 doesn't. A page snapped to A4 is exactly 595.28 × 841.89 pt.

### A12. Homography

**Source:** in-house (`scancraft-geom`); Heckbert (1989), §2.2.3.

**Idea.** A homography is the 3 × 3 matrix that maps points on one flat surface to points on another, the way a camera sees a tilted plane. Here it maps each pixel of the flat output page back to the place in the photo it comes from.

Map the unit square onto the quadrilateral, with (0, 0) → TL = (x0, y0), (1, 0) → TR = (x1, y1), (1, 1) → BR = (x2, y2) and (0, 1) → BL = (x3, y3). L is the photo's long side, as in A11.

```text
Σx = x0 − x1 + x2 − x3          Σy = y0 − y1 + y2 − y3

if |Σx| < 1e−9·L and |Σy| < 1e−9·L:        (parallelogram: affine case)
    a = x1 − x0    b = x2 − x1    c = x0
    d = y1 − y0    e = y2 − y1    f = y0    g = h = 0
else:
    Δx1 = x1 − x2    Δx2 = x3 − x2    Δy1 = y1 − y2    Δy2 = y3 − y2
    den = Δx1·Δy2 − Δx2·Δy1
    g = (Σx·Δy2 − Δx2·Σy) / den
    h = (Δx1·Σy − Σx·Δy1) / den
    a = x1 − x0 + g·x1    b = x3 − x0 + h·x3    c = x0
    d = y1 − y0 + g·y1    e = y3 − y0 + h·y3    f = y0

x(u, v) = (a·u + b·v + c) / (g·u + h·v + 1)
y(u, v) = (d·u + e·v + f) / (g·u + h·v + 1)
```

For an output page of W_o × H_o pixels, output pixel (*i*, *j*) uses u = (*i* + ½)/W_o and v = (*j* + ½)/H_o. Everything is computed in f64. `den` is zero only for a degenerate quadrilateral, which A9 and the interface already reject; it is still checked and returned as an error.

Rotation by q clockwise quarter turns is folded in before the mapping is computed: new corner k = old corner (k − q) mod 4, and W_o and H_o swap when q is odd. The output comes out already rotated, with no extra pass.

**Tests:** the four corners map exactly (error under 1e−9 px); affine inputs take the parallelogram branch; mapping a point and back through the inverse matrix (its adjugate) returns it within 1e−9; folding a quarter turn gives the same pixels as rotating the output.

### A13. Perspective warp

**Source:** in-house; cubic convolution from Keys (1981).

For each output pixel, compute (x, y) with A12, evaluated incrementally along each row because the numerators and denominator are linear in u. A flattened page uses A28's mapping instead, evaluated incrementally along each column, or along each row when the sheet bends down the page (A28 step 5). Then:

- If (x, y) falls outside [0, W] × [0, H], the pixel is white.
- Otherwise, sample with Catmull–Rom bicubic interpolation (Keys, a = −0.5) at index position (x − ½, y − ½), using the 4 × 4 neighbouring pixels with edge pixels repeated at the border, and clamp the result to [0, 1]:
  ```text
  w(t) =  1.5·|t|³ − 2.5·|t|² + 1             for |t| ≤ 1
  w(t) = −0.5·|t|³ + 2.5·|t|² − 4·|t| + 2      for 1 < |t| < 2
  w(t) =  0                                    otherwise
  ```

Output size: the native size from A11 when the output DPI is at or below the native density; otherwise (Black & White only, A18) the output size directly. Interpolation runs on sRGB-encoded values, in parallel row bands.

**Tests:** an identity mapping reproduces the image exactly; a synthetic checkerboard warped and then warped back reaches ≥ 40 dB PSNR away from its edges; areas outside the photo are white.

### A14. Resample to the output resolution

**Source:** in-house; Lanczos-3 (Duchon 1979; Turkowski 1990).

When the native density exceeds the output DPI by more than 2%, the warped page shrinks to the output size with a separable Lanczos-3 filter. For a scale k = out/in < 1, each output sample takes the source samples within 3/k px of its centre, weighted by L(k·t), where L(t) = sinc(t) · sinc(t/3) for |t| < 3 and sinc(t) = sin(πt)/(πt). Weights are normalised to sum to 1. The horizontal pass runs first, then the vertical pass, in f32, then values are clamped and rounded to 8 bits.

Output DPI: Original, Color Document and Grayscale use the smaller of the preset's DPI and the native density; Black & White always uses the preset's DPI (A18).

**Tests:** a constant image stays constant; a zone plate shows no visible aliasing; a grating at 0.3 cycles per output pixel keeps at least 90% of its contrast, and one at 0.4 cycles keeps at least 70% (the kernel's own response is 0.99 at 0.3 cycles and 0.82 at 0.4, so no Lanczos-3 filter could keep 90% at 0.4).

### A15. Upright

**Source:** Apple Vision text recognition in its fast mode on macOS; the fallback is `detect_orientation`, ported from `pdfcraft-ocr`, which compares the ink profiles of rows and columns.

Runs once when Correct Perspective is applied, if "Rotate pages upright automatically" is on.

1. Make a 1024 px Grayscale version of the page (A17 at default strength).
2. For each of 0°, 90°, 180° and 270°, rotated exactly: run `VNRecognizeTextRequest` with recognition level Fast and language correction off. Score = the sum over results of the top candidate's confidence × its character count.
3. Turn the page to the best rotation only if its score is at least 1.5× the score at 0° and it read at least 20 characters. Otherwise leave it.
4. If Vision is unavailable, or no rotation read 20 characters (fast mode reads fewer scripts), use `detect_orientation`, which requires a margin of 1.3× between row and column profiles. If it is unsure, leave the page as it is.

The user's Rotate buttons always override the result.

**Tests:** 40 turned synthetic text pages, 10 per rotation, all come out upright; a page without text stays as it is.

### A16. Even Light: lighting and shadow correction

**Source:** in-house. Design, parameters and measurements from pdfcraft-android #298, whose output came within a 2.9% mean luminance difference of a commercial scanner app's one-tap result on ten photo pairs. Normalized convolution from Knutsson and Westin (1993); minimum and maximum filters from van Herk (1992) and Gil and Werman (1993).

**Idea.** A photographed page is the paper's own lightness multiplied by the light falling on it. Even Light estimates how bright the bare paper would look at every point (the "background"), using only pixels it is confident are bare paper, and divides the photo by that estimate. The paper becomes evenly white, and ink keeps its darkness relative to the paper around it. Photos, logos, dark boxes and fingers are kept out of the estimate, so they aren't mistaken for shadows and bleached.

Input: the warped page I (sRGB-encoded, f32) at its output resolution. Steps EL1–EL8 run on an analysis copy whose long side is 1000 px.

1. **EL1, analysis copy.** A3 to a long side of 1000 px, then a Gaussian with σ = 0.6 px. Y = luma.
2. **EL2, stroke width.** Mark pixels darker than 0.8 × the mean of their 31 × 31 neighbourhood. On every 6th row, collect horizontal runs of marked pixels 1–59 px long. s = P75 of the run lengths, or 3 if there are fewer than 50 runs. Closing radius r = clamp(round(2.5 · s), 4, 24) px.
3. **EL3, first background b0.** Per channel: 3 × 3 median, then a closing (A19) with an octagon of radius r, then a Gaussian with σ = 1.5 px. Dark marks thinner than about 2r disappear, while shadows, which are wide, stay.
4. **EL4, dark objects.** On a copy four times smaller, close luma(b0) with a radius of 12% of that copy's long side, blur with σ = 3 px, and scale back up bilinearly. The result, bl, is the paper's level including large-scale shading. Let depth = luma(b0) / max(bl, 0.03). Examine each 8-connected component of depth < 0.8 whose area is between 12 px and 30% of the page:
   - d50 = the median depth inside it;
   - sharp = P75 of the Sobel gradient (÷ 8) of luma(b0) within 2 px of its boundary, divided by max(drop, 0.02), where drop = median(bl) − median(luma(b0)) inside;
   - cd = the largest channel difference between the median colours inside it and in a band 2–6 px outside it, each colour first divided by its own largest channel.

   The component is an object if d50 < 0.5, or if sharp ≥ 0.12 and it either doesn't touch the page's border or has cd > 0.2. Any other component is a shadow.
5. **EL5, warm objects** (skin, wood and cardboard at the page's edge). On b0, let rb = ln(max(R, 0.01) / max(B, 0.01)) and sat = (max − min)/max over the channels. Lit paper is the pixels with luma(b0) at or above its P85. Candidates: rb > ln 1.22 + median of rb over lit paper, and sat > 0.10 + median of sat over lit paper; open them with a 3 × 3 square and keep components covering at least 0.1% of the page. On a copy of b0 blurred with σ = 1: jump = the median rb inside minus the median rb in a band 3–7 px outside; sharp_rb = P75 of the gradient of rb within 3 px of the boundary, divided by max(|jump|, 0.03). The component is an object if jump ≥ 0.25 and sharp_rb ≥ 0.08.
6. **EL6, masks.** O = the union of the objects from EL4 and EL5, kept as the photo mask for A17 and A18. O_d = O dilated by an octagon of radius r + 2. Ink = (Y / max(luma(b0), 0.02) < 0.88), dilated by 3 × 3. Paper = not ink and not O_d.
7. **EL7, paper colour under the local light, B.** Normalized convolution (A19) of the paper pixels, per channel: B_σ = G_σ ∗ (paper · I) / G_σ ∗ paper. Try σ1 = max(2, 0.5 · r), then 4σ1, then 16σ1. At each pixel, use the first scale whose local paper share, G_σ ∗ paper, exceeds 0.15, 0.02 and 0.003 respectively. Failing all three, use the median paper colour of the page.
8. **EL8, keep hard shadow edges.** Outside O_d, B = min(B, b0) per channel. The smooth estimate would blur the edge of a hard shadow into a halo; b0 keeps it sharp.
9. **EL9, divide.** Scale B to the page's resolution bilinearly, with pixel centres aligned. Per channel, n_c = clamp(I_c / max(B_c, 0.02), 0, 1.3). The paper becomes 1.0 in every channel, which also removes the light's colour cast. With Keep paper colour on, n_c ← n_c · P_c / max(P), where P is the median of B over lit paper.
10. **EL10, neutral black.** Take the darkest 2% of pixels by luma; let D be their median colour and l = luma(D). Per channel, off_c = clamp(D_c − l, −0.08, 0.08), and n_c ← n_c − off_c · clamp((1 − Y)/(1 − l), 0, 1). This turns ink black instead of the bluish or brownish black cameras record, without changing the paper. The ±0.08 limit keeps a page printed only in blue ink blue.

Output: n, the lighting-corrected RGB in f32, and the object mask O.

**Tests** (synthetic, adapted from #298, measured on the Color Document look at default strength): with flat light, paper ≥ 245 on at least 97% of paper pixels and the ink median ≤ 30. A 2.5 : 1 lighting gradient leaves a spread of paper brightness ≤ 3%. A hard shadow at ×0.6: the step across its edge ≤ 4 levels, the mean halo ≤ 3 levels, and 95% of paper within 12 levels of the paper median. Dappled shade: paper ≥ 245 on at least 90%. Black bars that don't touch the border stay darker than 60. A skin patch at the page's edge keeps its luminance within 15% and its hue within 8°. White paper under coloured light: |R − B| ≤ 4 levels. A pale pencil line (0.70 on 0.93) ends up at least 40 levels darker than the paper. A blank page has a standard deviation ≤ 2 levels. One thread and eight threads give identical bytes.

### A17. Looks: Original, Color Document, Grayscale

**Source:** in-house; tone and colour formula from pdfcraft-android #298.

**Original:** the warped page, unchanged, with no strength control.

**Color Document** (strength s, default 60; k = s/100):

1. **Sharpen.** Y = luma(n). Y_s = unsharp mask of Y (A19: σ = 0.8 px @300 DPI, amount 0.5, threshold 0.01). Then n_c ← n_c + (Y_s − Y) and Y ← Y_s.
2. **Tone.** bp = 0.42 · k, wp = 1 − 0.05 · k, γ = 1 + 0.67 · k. At s = 60 these are a black point of 0.25, a white point of 0.97 and a gamma of 1.4.
   T = clamp((Y − bp)/(wp − bp), 0, 1)^γ.
3. **Colour.** w = smoothstep(0.04, 0.12, max_c n_c − min_c n_c), where smoothstep(e0, e1, x) = t²(3 − 2t) with t = clamp((x − e0)/(e1 − e0), 0, 1). sat = 1 + 0.33 · k. out_c = clamp(T + sat · w · (n_c − Y), 0, 1). Real colour keeps its strength, while faint tints under 0.04 vanish, so paper and black ink become neutral.
4. **Photo regions.** m = O blurred with σ = 2 px at analysis scale and scaled up. out ← (1 − m) · out + m · clamp(n, 0, 1). Photos keep their corrected colours without the document curve.
5. **Low strengths.** Strength 0 returns the warped page unchanged. Strengths 1–19 blend the strength-20 result with the warped page by s/20, so the slider has no jump near zero.

**Grayscale:** the same steps with sat = 0, so the output is T, stored as one 8-bit channel.

**Why not CLAHE.** Contrast Limited Adaptive Histogram Equalization stretches the histogram of each tile of the image. In a tile that is 99% paper at 250 ± 3, CLAHE with a clip limit of 2 maps the paper to about 235–253: the paper turns grey and its grain grows threefold, while the text barely changes. After Even Light, the remaining job (white paper, black ink, faithful colour) is the same everywhere on the page, so one global curve does it without seams between tiles.

**Tests:** the A16 synthetic tests; strength 0 gives bytes identical to the warped page; the Grayscale output equals T exactly.

### A18. Black & White

**Source:** in-house. The method pdfcraft-android #300 shipped after measuring it against Sauvola and a global threshold; related to Bernsen (1986). Ordered dithering from Bayer (1973).

**Idea.** Each pixel is compared with the midpoint between the lightest value nearby (the paper) and the darkest value very nearby (the stroke). Where those two are close, there is no ink edge to judge, and the pixel is paper unless it is very dark.

Input: the luma of n from A16 at the Black & White output DPI D: 300, or 400 with Best quality. If the page's native density is under 200 DPI, the warp renders at twice the size (bicubic) first.

1. **Smooth.** Gaussian, σ = 0.7 px @300 DPI.
2. **Paper level.** M = maximum over a square of side w_p = odd(round(31 · D/300)), about 2.6 mm, a little more than the height of a lowercase x.
3. **Stroke level.** m = minimum over a square of side w_i = odd(round(9 · D/300)), about 0.76 mm.
4. **Gate.** g = 0.04 + 0.16 · (1 − k), with k = strength/100. The default strength of 55 gives g ≈ 0.112.
5. **Decide.** A pixel is ink if Y < 0.25 (solid dark areas), or if M − m ≥ g and Y ≤ (M + m)/2.
6. **Photos.** Inside photo regions (O from A16, components of at least 0.5% of the page), use an 8 × 8 Bayer ordered dither of the Grayscale look instead: ink if T < (B8[*i* mod 8][*j* mod 8] + 0.5)/64. The Bayer matrix is built recursively: B1 = [0], and B2n = [[4·Bn, 4·Bn + 2], [4·Bn + 3, 4·Bn + 1]].
7. **Despeckle.** Remove 8-connected ink components of at most round(3 · (D/300)²) px that have no other ink within round(2 · D/300) px.

Output: 1 bit per pixel, 1 meaning ink.

**Measured** in pdfcraft-android #300 on synthetic 300 DPI pages, as F-measure of printed text and the share of a pale pencil line kept:

| Method | F-measure | Pencil kept |
|---|---|---|
| Lightest/darkest midpoint (this method) | 0.989 | All of it, away from other print |
| Sauvola, k = 0.34 | 0.959 | 10% |
| Sauvola, k = 0.2 | 0.941 | 22% |
| Sauvola, k = 0.1 | 0.932 | 78% |
| One global threshold | 0.935 | 24% |

No single Sauvola setting kept both a pale pencil line and a bold heading.

**Known limit:** inside a solid mid-grey or coloured area wider than w_p, the gate sees no contrast, so the area turns white unless it is darker than 0.25.

**Tests:** F-measure ≥ 0.96 on synthetic pages at 300 DPI and ≥ 0.90 at 200 DPI; a pale pencil line (contrast 0.2) and a bold heading both recalled at ≥ 0.85; blank paper with σ = 6 noise has fewer than 0.01% black pixels; a solid black box stays at least 97% black; full stops and the dots on i survive at 150, 200 and 300 DPI; a dithered photo's mean darkness stays within 10% of the source.

### A19. Shared helpers

**Source:** in-house, except where a crate is named.

- **Minimum and maximum filters** (van Herk 1992; Gil and Werman 1993). Three comparisons per sample for any odd window length w. Pad the signal with (w − 1)/2 repeated edge samples on each side and split it into blocks of w samples. Let g be the running maximum from each block's start and h the running maximum from each block's end. The window starting at padded index *i* has maximum max(h[*i*], g[*i* + w − 1]). Square windows use a horizontal pass, then a vertical one; minimum filters swap max for min.
- **Octagon of radius r**, standing in for a disk: maximum (or minimum) passes along 0° and 90° with half-length a = round(0.4142 · r), then along 45° and 135° with half-length b = round(0.2929 · r) in diagonal steps. A closing is the maximum passes followed by the minimum passes.
- **3 × 3 median:** a sorting network of 19 comparisons per pixel and channel (Paeth 1990).
- **Unsharp mask:** Y_s = Y + amount · (Y − G_σ ∗ Y) where |Y − G_σ ∗ Y| exceeds the threshold; Y_s = Y elsewhere.
- **Connected components:** `imageproc` `region_labelling::connected_components` with 8-connectivity.
- **Normalized convolution** (Knutsson and Westin 1993): (G_σ ∗ (c · I)) / (G_σ ∗ c) for a 0/1 confidence mask c, with both convolutions done by A4.
- **Oklab** (Ottosson 2020): linearise sRGB, multiply by Ottosson's published matrix M1, take cube roots, multiply by M2.

### A20. Text recognition

**Source:** Apple Vision `VNRecognizeTextRequest` through `objc2-vision` 0.3.2 in `scancraft-apple`. The fallback, in the command-line tool only, calls ocrs 0.13.1 on the rten 0.26 runtime directly, in the way PdfCraft's `OcrsRecognizer` does: Latin script only, with models licensed CC-BY-SA-4.0. Both implement the `Recognizer` trait in `scancraft-core`.

**When:** as soon as a page is added, in the background at the lowest priority; again if its corners or rotation change; and at export for any page still without text.

**Input:** the Grayscale look at default strength and the page's output resolution, whichever look the user chose. Recognition then doesn't depend on the look, and it never sees the 1-bit image, which has lost the grey edges recognisers rely on.

Steps on macOS:

1. Encode the image as an 8-bit grey PNG with fast compression, as `NSData`.
2. Create a `VNImageRequestHandler` with `initWithData:options:` and a `VNRecognizeTextRequest` with `new`. Set the recognition level to Accurate and language correction from Settings (on by default). For Automatic language, call `setAutomaticallyDetectsLanguage(true)`; otherwise `setRecognitionLanguages` with the chosen list. Call `setMinimumTextHeight(max(0.0025, 12 / page height in px))`, so 6-point text at 300 DPI is still read.
3. Call `performRequests:error:` on the Vision thread inside an autorelease pool, with a 20 s limit per page.
4. For each `VNRecognizedTextObservation`, in the order Vision returns them (which follows the layout's reading order), take the top candidate's `string` and `confidence`. Split the string into words at Unicode word boundaries (UAX #29, `unicode-segmentation`) and drop whitespace. For each word, ask `boundingBoxForRange:error:` with the word's UTF-16 range for its quadrilateral, convert it from Vision's normalised bottom-left coordinates to page pixels (x = u · W_p, y = (1 − v) · H_p), and take the enclosing rectangle. If Vision gives no box, split the line's box in proportion to the words' UTF-16 lengths.
5. Store the result in the recipe as `Vec<Line>` of `Word { text, rect: [left, top, right, bottom], confidence, source: "vision" }`, together with the macOS version.

**Fallback:** if Vision fails on a page, the Mac app exports that page without a text layer and says so; the macOS build doesn't bundle the ocrs models. The command-line tool on Linux uses ocrs.

**Tests:** on synthetic 300 DPI text pages, a character error rate ≤ 1% with Vision, and word boxes that overlap the true word boxes with IoU ≥ 0.7 for at least 95% of words. Across the reference set's categories, recognition on the Grayscale look is never worse than on the unprocessed warped page. The proportional fallback is tested through a fake Vision layer.

### A21. Invisible text layer

**Source:** ported from PdfCraft (§5.3): `pdfcraft-ocr::text_layer`, `pdfcraft-fonts::TextPlan` and `pdfcraft-edit::add_type3_fonts`, writing through `pdf-writer`. The steps keep PdfCraft's names.

**Idea.** PDF can draw text in "rendering mode 3", which places the characters but paints nothing. Viewers still search, select and copy that text. Placing each recognised word exactly over its ink makes an image of a page searchable.

1. Map page pixels (x right, y down) to PDF points (x right, y up): to_user(x, y) = [x · W_pt/W_px, H_pt − y · H_pt/H_px].
2. Call `PlacedWord::place(&word, to_user)` for every word.
3. `text_layer(&placed, &plan)` returns the content stream: `/OCR BMC q BT 3 Tr … ET Q EMC`. Each word gets its own text matrix, which stretches the word's natural width to its box, with a size taken from the height of its ink. The `q … Q` pair stops rendering mode 3 from leaking into anything drawn later (pdfcraft #3033).
4. Fonts: words whose characters all exist in the WinAnsi encoding use standard Helvetica. Other words use Type3 fonts with empty glyphs and Unicode mappings (ToUnicode), so text in any script copies out correctly without embedding a real font.

**Tests:** text extracted from the PDF (Poppler's `pdftotext -bbox`) contains at least 99% of the recognised words, in order, each within its word box at IoU ≥ 0.8; a page renders pixel-identically with and without the layer; no `Tr`, `Tf` or `Tm` sits outside a `q … Q` pair.

### A22. Image encoding

**Source:** `craft-jpeg::encode_baseline`, which writes optimised Huffman tables for each image and supports 4:4:4, 4:2:2 and 4:2:0 colour sampling; `craft-stream-codecs::encode_jbig2_generic`, which writes one lossless JBIG2 generic region.

| Look | Format | Settings (Small / Balanced / Best) |
|---|---|---|
| Original, Color Document | JPEG, 3 channels | Quality 70 at 4:2:0 / 82 at 4:2:0 / 92 at 4:4:4 |
| Grayscale | JPEG, 1 channel | Quality 70 / 82 / 92 |
| Black & White | JBIG2 generic, 1 bit | Lossless. Flate-compressed 1-bit rows if JBIG2 encoding fails. |

Each page is encoded once, and those bytes go into the PDF unchanged. No EXIF or ICC segments are written.

**Tests:** JBIG2 decodes back exactly (with `craft-stream-codecs`); JPEG at quality 82 reaches ≥ 38 dB PSNR on the reference page. Size targets on the reference A4 text page at 300 DPI, to revise once measured in M1: Color Document ≤ 600 KB, Grayscale ≤ 450 KB, Black & White ≤ 100 KB.

### A23. PDF assembly

**Source:** `pdf-writer`, with image pages ported from `pdfcraft-create`'s `from_images` (§5.3); the file is written through `craft-atomic`.

For each page, in tray order:

- MediaBox [0 0 W_pt H_pt], from the physical size in A11, or from the fixed size chosen in Export Options with the image fitted and centred, never stretched.
- One image XObject holding the bytes from A22: `/DCTDecode` for JPEG (`/DeviceRGB` or `/DeviceGray`), or `/JBIG2Decode` for Black & White (1 bit, `/DeviceGray`, with bit polarity as ISO 32000-2 §7.4.7 defines it, confirmed by a test that black text renders black).
- Page content `q W_pt 0 0 H_pt 0 0 cm /Im1 Do Q`, followed by the text layer stream from A21.

Document information: Title (the file name without its extension), Producer ("ScanCraft *version* (algorithms *ALGO_VERSION*)"), CreationDate and ModDate. Nothing from the photos' EXIF.

The PDF is written to a temporary file next to the destination, flushed to disk, then renamed over the destination. A failed export never leaves a partial PDF.

**Tests:** `qpdf --check` reports no errors; rendering each page with `craft-pdf`'s rasteriser matches its encoded image within a mean of 1 level; an export killed halfway leaves the destination untouched; the release checklist opens the PDF in Preview, Chrome and Firefox.

### A24. PDF import

**Source:** `craft-pdf` with the `read` and `raster` features: PdfCraft's patched Hayro reader and rasteriser, shared through craft-libs.

1. Open the file with `craft-pdf`'s bounded reader. A password-protected PDF is skipped with the message "This PDF is password-protected."
2. Render each page at 300 DPI, honouring its `/Rotate`. If that exceeds 40 megapixels, lower the DPI until it fits. Composite the premultiplied RGBA result over white.
3. Each page becomes an import item. Its corners default to the full page (Use Full Image), because PDF pages are usually flat already; Auto-Detect is one click away.

At most 500 pages are imported from one PDF; the message says how many were left out.

**Tests:** a three-page PDF adds three items in order; a rotated page arrives upright; encrypted, truncated and 10 000-page PDFs fail or stop cleanly.

### A25. Curved sides

**Source:** in-house. The path search is dynamic programming over a band around each side, the approach Montanari (1971) introduced for finding curves in noisy pictures.

**Idea.** A curved side of a page is still a long step in brightness, but no straight line follows it. A25 looks across the side every 2 px along it, over a band that is narrow at the corners and wide in the middle. It then finds the path that stays on the strongest step while changing course as little as possible.

Input: the corners from A10, each side's polarity from A10 step 4, and the level of A10's luma pyramid whose long side is closest to 2048 px. A25 runs on every side except border sides (A9) and the spine of a spread (A30).

1. **Band.** The side's chord runs straight from its first corner to its second; len is its length in pixels of this level, and n is its outward normal. Columns k = 0 … N − 1 lie every 2 px along the chord, skipping 2% of it at each end, and t_k is a column's position as a fraction of the chord. Column k samples the luma along n at offsets d from −r_k to +r_k, every 1 px, with r_k = 6 + 0.32 · len · t_k · (1 − t_k). The band reaches 6 px either side of the chord at the corners, and 6 px plus 8% of the side's length at its middle.
2. **Step strength.** Smooth each column's samples with a Gaussian (σ = 1 px) and differentiate them with central differences. Keep only steps with the side's polarity, setting the rest to 0. Divide by the 95th percentile of the band's values and clamp to [0, 1]. Call the result s_k(d).
3. **Best path.** Choose one offset d_k per column to maximise Σ s_k(d_k) − 0.05 · Σ |d_{k+1} − d_k|, with |d_{k+1} − d_k| ≤ 2 px. A forward pass keeps, for every offset, the best score of any path ending there, and a backward pass reads the path off. Ties go to the smaller |d|.
4. **Outermost path.** Repeat step 3 over the offsets at least 3 px outside the first path. If the new path's mean strength is at least 70% of the first path's, it replaces it. This is A10's rule carried over to curves: a printed frame just inside a sheet's edge can be as strong as the edge. With Book Spreads on, the second search looks inward instead, because the edges of the pages under a book's top page lie just outside it.
5. **Samples.** A column is supported when s_k(d_k) ≥ 0.25. Refine its d_k with a parabola through the strengths at d_k − 1, d_k and d_k + 1. Its sample is the photo position of chord(t_k) + d_k · n at full resolution, with weight s_k(d_k).
6. **Straight or curved.** Fit a straight line through the supported samples by weighted total least squares. The side's bow is the largest distance from that line, after a running median over 15 columns, as a fraction of len. The side is curved when its bow is at least 0.3% and at least 2 px, and at least 60% of its columns are supported.

Output, per side: Straight or Curved, the bow, and at most 64 samples spread evenly over the supported columns. A27 uses the samples of straight sides as well.

A column holds at most about 340 offsets, so tracing a side, both searches included, takes under 4 million additions.

**Tests:** synthetic pages whose top and bottom edges bow by 0.5–6% of their length, starting from A10's corners: at least 95% of the samples within 1.5 px of the true edge, and every such side marked curved. Straight sides with 1 px of blur and noise σ = 6 levels: marked straight in at least 98% of cases. A ruled frame 12 px inside a curved edge: the edge is chosen. A book page with the edges of the pages beneath it showing: with Book Spreads on, the top page's edge is chosen.

### A26. Text lines

**Source:** a port of the strip tracker of pdfcraft-android #288 and #303 (`straightness::text_lines`, MIT OR Apache-2.0), with ink found by A18's local rule instead of one threshold for the whole page.

**Idea.** On a page of text, each line is a horizontal band of ink. A26 cuts the straightened page into narrow vertical strips, finds the bands of ink in each strip, and follows each band from strip to strip. The result is a few dozen points along the middle of every line of text.

1. **Analysis copy.** Straighten the page by its corners (A12, A13) to a long side of 1600 px. Sample from the level of A10's pyramid that has at least that many pixels across the page, so the copy isn't aliased. Take its luma.
2. **Ink.** A18 steps 1–5 at strength 55, at the copy's density: its pixels per inch at A11's page size. Under the shade of a hand or of a book's gutter, one threshold for the whole page loses the lines; A18's local rule keeps them. pdfcraft-android saw the same and evened the light first.
3. **Pieces of lines.** Cut the copy into 32 vertical strips of equal width. In each strip, count the ink pixels in every row and smooth the counts over 3 rows. Each run of rows whose count exceeds 15% of the strip's highest count, and which is at least 3 rows tall, is a piece of a text line. Its height is the centre of mass of the raw counts over the run.
4. **Following.** Go through the strips from left to right. Pair the open lines with the next strip's pieces, nearest heights first. A pair joins when the two heights are within 60% of that strip's median gap between neighbouring pieces. A line may skip at most 2 strips in a row.
5. **Keeping.** Keep the lines seen in at least 5 strips whose first and last points are at least 30% of the copy's width apart, up to 120 lines. Each point is a strip's centre column and the line's height there.
6. **Direction.** Repeat steps 3–5 on the copy turned by a quarter turn. Use the direction whose lines are longer in total, if they are at least 1.5 times as long as the other direction's; otherwise keep the first. The text then runs either across or down the page.
7. **Photo positions.** Map every point to the photo through the A12 mapping that made the copy. Each point's uncertainty is σ = max(1.5 px, 0.12 × the median piece height), converted to photo pixels.

Output: the lines, as lists of photo positions with their σ, and the text's direction. Fewer than 4 lines means the page offers no text evidence.

On the synthetic book pages behind A27's figures, lines in the corner-straightened copy slope by at most 0.11 at the 95th percentile and 0.21 at worst, which 32 strips follow comfortably. A sheet curled through 80° reaches 0.6; there the outline carries the fit (A27).

**Tests:** a synthetic page with 30 lines of 10 pt text, bent so that its lines slope by up to 0.3 in the copy: at least 27 lines followed, with points within 0.15 x-heights of the line's true middle. Two columns with aligned baselines: each line is followed across both. A page turned sideways: the direction is found. A photo, a blank page and noise each give fewer than 4 lines.

### A27. Sheet fit

**Source:** in-house. Modelling the page as a bent sheet and fitting it to the page's text lines comes from Zucker (2016, MIT) and pdfcraft-android #303 part B (MIT OR Apache-2.0); Liang, DeMenthon and Doermann (2008) and Kim, Koo and Cho (2015) fit related models to text. ScanCraft changes four things:
- The fit runs in the photo, with the camera's focal length.
- The page's outline is evidence alongside its text.
- The profile is a spline whose length gives the page's true width.
- The sheet may bend in either direction.

The solver is Levenberg–Marquardt (Levenberg 1944; Marquardt 1963) with Nielsen's (1999) damping rule. It removes the per-point unknowns with a Schur complement, as bundle adjustment does (Triggs et al. 2000). The starting pose comes from the corners' homography (Zhang 2000).

**Idea.** Paper bends but doesn't stretch. Think of an open book's page, a curled receipt or a letter folded in three: each is a sheet bent in one direction. Straight lines, called rulings, run across such a sheet, all parallel to the axis it bends around. Every cut at right angles to them shows the same curved profile. A27 finds that profile and where the sheet sits in front of the camera. It matches the sheet's text lines and edges, as the camera would see them, to where the photo shows them.

**Model.** Each point of the page has two coordinates in [0, 1]: t along the profile and b along the rulings. In the sheet's own frame, with its origin at the top-left corner, and with the camera of §7.8:

```text
bend across (rulings run down the page, as in a book):            P(t, b) = ( c·t,  b,    −c·η(t) )
bend down   (rulings run across, as in a letter folded in three): P(t, b) = ( b,    c·t,  −c·η(t) )
in camera coordinates:  Q = R·P + T
```

- The rulings have length 1, which fixes the scale. c is the length of the profile's chord, the straight line between its two ends.
- η(t) is the profile's height toward the camera, as a fraction of c. It is the natural cubic spline through 12 knots at t_k = (1 − cos(πk/11))/2 for k = 0 … 11, held at 0 at both ends, with 10 free values. The knots crowd toward the ends, where a book page bends most sharply.
- The four corners are (t, b) = (0, 0), (1, 0), (1, 1) and (0, 1) in bend-across mode, with t and b swapped in bend-down mode. They lie on a flat rectangle in space, which is why A11's focal length estimate from the corners still holds for a bent page.

**Unknowns:**
- the rotation R (3 numbers, updated as a rotation vector) and the translation T (3);
- c and the 10 profile values;
- the focal length f, when EXIF didn't provide it;
- one coordinate per text line, the one that stays constant along it: for text running across the page, b in bend-across mode and t in bend-down mode; for text running down the page (A26 step 6), the reverse;
- one coordinate per point, giving its position along its line or side.

**Evidence.** Each item gives a residual e = (projected position − observed position) / σ, in two dimensions.

| Evidence | Where it lies on the sheet | σ |
|---|---|---|
| Text-line point (A26) | On its line: the line's shared coordinate, plus the point's own position along the line | From A26 |
| Side sample (A25) | On its side, with its own position along it. In bend-across mode: top b = 0, bottom b = 1, left t = 0, right t = 1. In bend-down mode t and b swap. | 1.5 px |
| A side the user bent | 64 points along the curve through its corners and bend handles, in place of that side's A25 samples | 1 px |
| Corner | Fixed | 2 px detected; 0.5 px placed by the user; 4 px at a spine; 8 px outside the photo |

**Cost.**

```text
F = ½ Σ ρ(|e|²)  +  ½ · 0.5 · ∫₀¹ η''(t)² dt  +  ½ · ((ln f − ln f₀) / 0.3)²
ρ(s) = 4 · ln(1 + s/4)
∫₀¹ η''(t)² dt = Σ_k h_k · (M_k² + M_k·M_{k+1} + M_{k+1}²) / 3
```

- ρ is the Cauchy loss with its scale at 2σ. A point far from where the sheet says it should be loses influence instead of dragging the fit.
- The middle term prefers a gentle profile where the evidence says little. h_k = t_{k+1} − t_k, and M_k is the spline's second derivative at knot k. M is a fixed linear function of the 10 free values, so the integral is exact. The weight of 0.5 comes from the prototype described below: weights from 0.1 to 1 did equally well, and 10 began to flatten real curls.
- The last term exists only when f is fitted. f₀ comes from A11, and the spread of 0.3 in ln f allows about 30% either way.

**Start.**

1. H is A12's homography for the corners, and K = [[f·L, 0, W/2], [0, f·L, H/2], [0, 0, 1]]. Let m1, m2 and m3 be the columns of K⁻¹·H. If the third component of m3 is negative, negate all three.
2. In bend-across mode λ = |m2| and c = |m1|/λ; in bend-down mode λ = |m1| and c = |m2|/λ. T = m3/λ. R is the rotation nearest to the matrix with columns m1/|m1|, m2/|m2| and their cross product, found by 20 iterations of X ← (X + X⁻ᵀ)/2 (Higham 1986).
3. η = 0. Every point's own coordinate comes from H⁻¹, and each line's coordinate is the median over its points.

**Solver.** Levenberg–Marquardt improves the estimate step by step. Each step solves a linearised version of the problem, damped so that the step stays short where the linearisation is poor.

1. At each iteration, linearise the residuals and recompute the Cauchy weights w = 1/(1 + |e|²/4).
2. Solve (JᵀWJ + μ·D) δ = −JᵀWe. J is the Jacobian, W holds the weights, D is the diagonal of JᵀWJ, and the bending and focal terms are added in.
   Each point's own coordinate enters only its own residual, so those unknowns are eliminated first (a Schur complement). That leaves a dense system of at most 18 + 120 unknowns, solved by Cholesky factorisation. A failed factorisation counts as a rejected step.
3. Compute the gain ratio ρ_g = (F before − F after) / (the decrease the linearisation predicted).
   - If ρ_g > 0, accept the step and set μ ← μ · max(1/3, 1 − (2ρ_g − 1)³) and ν ← 2.
   - Otherwise reject it and set μ ← μ · ν and ν ← 2ν.
   - Start with μ = 10⁻³ and ν = 2.
4. Rotation updates apply as R ← R · exp([δω]×), by Rodrigues' formula. Point coordinates stay within [−0.05, 1.05].
5. Stop after 60 iterations, when an accepted step lowers F by less than 10⁻⁷ of its value, or when μ exceeds 10¹². In the prototype, fits took at most 18 iterations, 9 at the median.

Sums run in a fixed order (§7.6), so the result doesn't depend on the number of threads.

**Three fits** start from the same point: a plane (η held at 0), bend across, and bend down. The bent fit with the lower F is the curved candidate. In the prototype, on 90 bent scenes, this picked the true bend direction every time.

A fit is invalid if any point lies behind the camera, c ≤ 0, f leaves [0.35, 4.0], the profile is steeper than 72° anywhere (|η'| > 3), or the solver failed.

Output: the plane fit and the curved candidate, each with R, T, c, η, f, F and the RMS residual for each kind of evidence.

**Why fit in the photo.** pdfcraft-android fitted a similar model to the page after straightening it by its corners, with an assumed camera. Straightening hides how far away the camera was, so the depth of the bend can't be told apart from the camera's distance. At 25° of tilt its fits found bends 0.6–2 times the truth (#303). Fitting in the photo, with A11's focal length, keeps that information.

The figures below come from a numerical prototype of this fit, run on synthetic evidence rather than rendered images. Bent sheets were projected by cameras with focal lengths of 0.6, 0.75 or 1.0 times the photo's long side, and tilts up to 25°. The projected points then received noise:
- 2 px of noise on text points, with 5% of them moved by up to 25 px;
- 1.5 px of noise on side samples;
- 2 px of error on the corners.

The error of placing the flat page's content was:

| Page shape, 40 scenes each | Corners only (A12) | Fitted after straightening | Fitted in the photo | The same, focal length fitted from 0.75 |
|---|---|---|---|---|
| Book page rising 25–65° from the spine | 1.43% | 1.42% | 0.08% | 0.09% |
| Sheet curled through 20–80° | 2.36% | 1.68% | 0.06% | 0.06% |
| Sheet sagging in the hand | 1.54% | 1.18% | 0.05% | 0.05% |
| Letter folded in three, folds of 10–25° | 1.14% | 0.93% | 0.11% | 0.11% |
| Flat sheet | 0.11% | 0.07% | 0.05% | 0.05% |

The error is the flattening error of §10.3: the 95th percentile, over the page, of how far each point of the flat page lands from its true place in the photo. It is given as a percentage of the page's diagonal in the photo, as the median over the scenes. The straightened fit was given the true proportions to straighten to. On a flat sheet the fit beats the corners alone only because the prototype's corners are off by 2 px, more than A10's tests allow; A29 leaves flat pages to A12.

The unrolled proportions (A28) came within 0.07–0.13% of the truth at the median and 0.53% at worst. With the focal length fitted, they came within 0.09–0.19% and 1.2%.

**Why the outline counts.** On the same book pages, text lines alone left a median error of 0.26%, the outline alone 0.08%, and both together 0.08%. Pages with little text still flatten. pdfcraft-android's fit used text only, so it couldn't flatten them.

**Why fit the focal length.** A wrong focal length misjudges how far away the page is, and with it how deep its curve is. In one test, f was held at the typical 0.75 for every scene while the true value was 0.6, 0.75 or 1.0. The median error on book pages rose from 0.08% to 0.55%, and a tenth of the scenes exceeded 2.5%. Fitting f with the page, from the same start, brought the median back to 0.09%. With an EXIF focal length (A11), f stays fixed.

**Tests:**
- The curved-page gates of §10.4 pass on synthetic scenes.
- A flat page's curved fit moves the page by less than 0.2% of its diagonal.
- The true bend direction is found on every bent scene.
- Each of the following keeps the median error under 0.15% (0.04–0.08% in the prototype): 15% of text points wrong, words shifted 3 px within their lines, or half of one side hidden.
- The solver converges on a known problem from a poor start. On degenerate input (all points on one line, a corner behind the camera) it returns an error, never a panic.
- One thread and eight threads give identical results.

### A28. Unrolling the sheet

**Source:** in-house.

**Idea.** Paper doesn't stretch, so a flattened page is as wide as its profile is long when measured along the paper, not as wide as the straight distance between the profile's ends. Each column of the flattened page is a ruling, a straight line on the sheet, so a whole column comes from one straight line in space.

1. **Length of the profile.** s(t) = c · ∫₀ᵗ √(1 + η'(τ)²) dτ, tabulated at 4097 evenly spaced values of t by the trapezoid rule. S = s(1) is the profile's unrolled length, in units of the ruling length.
2. **Proportions.** The page's width-to-height ratio is r = S in bend-across mode and r = 1/S in bend-down mode. It replaces A11 step 4's ratio; snapping and the physical size follow as in A11.
3. **Native size.** A11 step 7, with the side lengths measured along the fitted outline. Ŵ is the longer of the top and bottom sides and Ĥ the longer of the left and right sides, each the length of its projection in the photo, measured through 256 points.
4. **Mapping.** For output pixel (i, j) of a W_o × H_o page, u = (i + ½)/W_o and v = (j + ½)/H_o.
   - In bend-across mode, t = s⁻¹(u · S) and b = v. In bend-down mode, t = s⁻¹(v · S) and b = u.
   - s⁻¹ is linear interpolation in the table, which increases strictly.
   - Then Q = R · P(t, b) + T, and the camera (§7.8) gives the photo position.
   - Quarter turns are folded in by permuting the output axes, as in A12.
5. **Column by column.** In bend-across mode every output column has a single t, so its points lie on one straight line in space: Q = Q₀ + b · R · (0, 1, 0). Along the column, x and y are ratios of linear functions of b. They are evaluated incrementally, like A13's rows, with t, Q₀ and the coefficients computed once per column. In bend-down mode the same holds along rows.
6. **Fitted outline.** The photo positions of the sheet's four sides, 256 points each, form the outline Screen 2 draws for a flattened page. Its ends are the corners shown.

**Tests:**
- For a known sheet, the mapping matches direct evaluation within 10⁻⁶ px, and the incremental evaluation matches within 10⁻⁴ px.
- A plane (η = 0) gives A12's pixels within 1 grey level.
- A profile of known length gives S within 10⁻⁵.
- Text next to a book's spine comes out as wide as the same text in the middle of the page, within 2%.

### A29. When to flatten

**Source:** in-house. The final check on the finished page follows the quality guard of pdfcraft-android #302.

**Idea.** A page is flattened only when three things hold:
- The bent sheet explains its evidence clearly better than a flat one.
- The bent sheet moves the page by enough to matter.
- The flattened page's text lines are straighter than with the corners alone.

Input: A27's plane fit and curved candidate, A25's sides, A26's lines, and the page's Flatten setting.

- **Off:** the page uses A12's homography.
- **On:** the page uses the curved candidate if it is valid; otherwise A12, with the note of §4.8.
- **Auto**, the default, flattens only when all four checks pass:
  1. **Evidence.** The page has at least 4 text lines, or two opposite sides that A25 marked curved.
  2. **Size.** Take a 33 × 33 grid of page points, and for each, the distance between where the curved and plane fits place it. D95, the 95th percentile of those distances, is at least 0.2% of the page's diagonal in the photo.
  3. **Gain.** The curved fit's cost F is at most 0.97 times the plane fit's.
  4. **Straighter lines.** Straighten the page twice at the analysis size, once by its corners (A12) and once by the curved fit (A28), and run A26 on both. A page's straightness is the RMS distance of its line points from their own line's mean height, in analysis pixels. The check passes in either case:
     - the flattened page's figure is at most 0.9 times the other's, with at least 6 lines on both pages;
     - it is at most 0.5 times the other's, with at least 4 lines on both.

     With fewer than 4 lines on either page, the check passes only when two opposite sides are curved.
- **Record:** the decision, the check that settled it, D95, both costs, both straightness figures and both line counts. They feed Screen 2's tooltip, the session file and the evaluation report.
- Flatten is unavailable for Use Full Image and for pages imported from a PDF. Without the page's edges, the fit has nothing to pin its corners to.

**Measured** with A27's prototype on synthetic evidence, before check 4:
- No flat page passed checks 2 and 3. Among flat pages, the largest D95 was 0.16% and the largest gain under 1%.
- The test also used 90 mildly bent pages: curls of 4–20°, sags of 1.5–5°, and book pages rising 8–25° from the spine. 26 of them stayed flat, and on those the corners alone left at most 0.52% of the diagonal.

Real photos give rougher evidence, so milestone M2 tunes these constants on the reference set and records the figures beside them in the code.

**Why the final check.** The fit can be wrong in ways its own numbers don't reveal: lines linked across two columns, or a side traced along the edge of a hand. Measuring the finished page with the same tracker catches those. pdfcraft-android needed the same check for its flattening network, which moved flat pages by up to 2% of their diagonal.

**Tests:**
- Synthetic flat pages are left flat in at least 98% of cases.
- Synthetic bent pages on which the corners alone leave an error above 1% of the diagonal are flattened in at least 95% of cases.
- Off and On do what they say.
- An invalid fit under On falls back to A12, with the note.

### A30. Book spreads

**Source:** in-house. The gutter search and the split follow pdfcraft-android #303 part A (`book::find_gutter`, `book::split`, MIT OR Apache-2.0).

**Idea.** A photo of an open book holds two pages joined at the spine. The fold between them, the gutter, is a straight line in the photo, because it is a ruling of both pages. It is usually the darkest column of the spread, and no line of text crosses it.

A30 runs when Book Spreads is on (§4.3). With it off, steps 1–3 still run, and a gutter found with a confidence of at least 0.5 shows Screen 2's "Two pages? Split" button.

1. **Straighten** the spread by its outline (A12, A13) to a long side of 1000 px, and take its luma.
2. **Column profile.** Take the mean luma of each column over the middle 80% of the rows, and smooth it with a Gaussian whose σ is 1% of the width.
3. **Gutter.** Find the deepest local minimum within the middle 30% of the width that passes both tests:
   - it is at least 6% darker than the mean of the two halves' medians;
   - at most 10% of the text lines that A26 steps 2–5 find on this copy cross it.

   Its depth is how much darker it is, as a fraction of that mean. Its confidence is min(1, (depth − 0.06)/0.06).
4. **Spine.** Split the height into 8 bands. In each, find the profile's minimum within 3% of the width of the gutter. Fit x = a + b·y through the 8 points by least squares, drop points more than 1% of the width from the line, and fit again. The spine may lean by up to 8°.
5. **No gutter.** If no minimum qualifies but the spread's ratio, from A11 on the whole outline, is between 1.2 and 1.7, cut at the centre line with confidence 0. That range covers two pages side by side, from 5.5 × 8.5 in to Letter. Otherwise the photo stays one page.
6. **Split.** Map the spine to the photo through A12; it is straight there too.
   - Each page's outline is the spread's outline cut by the spine.
   - Each page's spine corners are where the spine meets the spread's top and bottom sides, along their A25 paths when those are curved.
   - Each page gives up 0.5% of the spread's width along the spine.
   - The spine side is marked as a spine: A25 doesn't trace it, A9 doesn't score it, and A27 uses only its corners, with σ = 4 px.
7. **Pages.** Each half becomes a page with the same source, its own corners and sides, and `spread: Some(SpreadHalf { half, spine })`. The two pages join the batch in the order set in Settings (§4.6).

The gutter's shadow needs no step of its own: A16 treats it as a soft shadow.

**Tests** (synthetic spreads, from #303):
- A gutter at 46% of the width is found within 0.5% of the width.
- A spine leaning from 45% to 51% of the width is followed.
- A single page with Book Spreads on stays one page.
- A two-page shape without a visible gutter is cut at the centre with confidence 0.
- Right to left puts the right page first.
- Deleting one page keeps the other and their shared source.

---

## 9. Performance budgets and limits

Reference machine: MacBook Air M1 with 8 GB, release build. Photo: 12 megapixels (4032 × 3024).

| Step | Budget |
|---|---|
| Decode HEIC, orient, convert to sRGB | ≤ 400 ms (JPEG ≤ 150 ms) |
| Work image and display copy | ≤ 40 ms |
| Detection, A5–A9, including Vision | ≤ 150 ms |
| Refinement, A10, for up to four candidates | ≤ 80 ms |
| Curved sides, A25, for up to four candidates | ≤ 30 ms |
| Text lines, A26, including the 1600 px analysis copy | ≤ 60 ms |
| Sheet fits, A27: plane, bend across and bend down | ≤ 40 ms |
| Flatten decision, A29, including the second analysis copy | ≤ 50 ms |
| Spread search and split, A30 | ≤ 20 ms |
| Warp and resample to A4 at 300 DPI | ≤ 150 ms; ≤ 200 ms for a flattened page |
| Look preview (1280 px) after a strength change | ≤ 80 ms |
| Even Light and look at full resolution | ≤ 600 ms |
| Encoding one A4 page | ≤ 80 ms |
| Text recognition (Vision, Accurate), one A4 page | ≤ 1.5 s, in the background |
| PDF writing per page, excluding encoding | ≤ 20 ms |
| UI frame | Never blocked by jobs; ≤ 8 ms for 120 Hz displays |

If pure-Rust HEIC decoding misses its budget, `scancraft-apple` decodes HEIC with ImageIO instead. M1 decides this from measurements.

A25–A27, A29 and A30 run in the detection job, ahead of the user, so Screen 2 opens with its curves drawn and the flatten decision already made.

**Memory:** at most 1.5 GB at peak for a 100-page session, with at most two decoded full-resolution photos in memory at once.

**Limits:** inputs up to 100 megapixels and 32 768 px per side; warped native size up to 40 megapixels; up to 500 pages per session and per imported PDF.

---

## 10. Testing and evaluation

### 10.1 Synthetic scenes

`scancraft-cli synth` renders a reference page with exact anti-aliasing: text at 6, 10, 12 and 36 pt, a pale pencil line, a solid black box, a coloured logo, a photo and a ruled table. It then places the page in a virtual camera view:

- Backgrounds: dark desk, light desk, procedural wood, checked tablecloth, white on white.
- Camera: focal length 0.6–1.2 × the long side, tilt 0–50°, any roll, corners up to 5% outside the frame.
- Light: a 2.5 : 1 gradient, a hard shadow at ×0.6 with a 3 px penumbra, dappled shade, a warm colour cast.
- Damage: a finger over an edge or a corner, blur up to σ = 1.5 px, noise up to σ = 6 levels, JPEG at quality 85.
- Bent pages (`--curved`): the page bent as a book page rising 25–65° from the spine, a sheet curled through 20–80°, a sheet sagging in the hand, a letter folded in three with folds of 10–25°, and two-page spreads with a gutter and a spine leaning up to 5°, all bent without stretching, under the same cameras with tilts up to 25°.

Each scene records its true corners, ratio and ink mask, and for bent pages the true position in the photo of every point of the flat page. A seed makes every set reproducible. CI runs the synthetic gates on every commit.

### 10.2 Real photos

- **SmartDoc 2015, challenge 1** (CC BY 4.0): video frames of documents on desks, with true corners. Used for detection.
- **ScanCraft reference set:** at least 90 hand-labelled photos (iPhone HEIC and Android JPEG) in 18 categories: dark desk, light desk, wood, patterned cloth, finger on an edge, finger on a corner, corner outside the frame, receipt, coloured paper, glare, low light, sideways page, and six bent ones: open book page, book spread (in both reading orders), curled receipt, letter folded in three, sheet held in the hand, magazine. For each bent photo, a flatbed scan of the same page supplies the reference text. Stored outside the repository, since they show people's documents, and referenced by hash.
- **UVDoc benchmark** (repository MIT): photographs of real bent and crumpled sheets with document images mapped onto them, with an exact grid saying where each point of the flat page lies. Used for the flattening error, and to compare with published methods. Crumpled pages lie outside A27's model and are reported separately.

### 10.3 Metrics

- **Jaccard index:** the overlap area divided by the union area of the found page and the true page, computed with SmartDoc's protocol.
- **Corner error:** the largest corner distance, as a percentage of the photo's diagonal.
- **No-touch rate:** the share of photos whose corner error is at most 0.5%, small enough that a user wouldn't adjust anything.
- **Flattening error:** the 95th percentile, over the page, of the distance between where ScanCraft places a point of the flat page in the photo and where it truly is, as a percentage of the page's diagonal in the photo. Synthetic scenes and the UVDoc benchmark.
- **Line straightness:** A29's measure, in pixels at 300 DPI, on the flattened page and on the page flattened by its corners alone.
- Ratio error, character error rate, and binarisation F-measure.

`scancraft-cli eval` writes every metric to a JSON report, which is attached to each release.

### 10.4 Release gates

| Gate | Target |
|---|---|
| SmartDoc 2015 challenge 1, mean Jaccard index, full pipeline on macOS | ≥ 0.97 |
| The same, classical detection only (A5–A7, A9, A10) | ≥ 0.93 |
| Reference set, no-touch rate | ≥ 85% |
| Reference set, "not found" | ≤ 5% |
| Synthetic bent pages, flattening error, median over scenes | ≤ 0.25% |
| The same, without an EXIF focal length | ≤ 0.35% |
| Synthetic flat pages that Auto flattens | ≤ 2% |
| Reference set, bent categories: median line straightness against the corners alone | At least 50% lower |
| Reference set, flat categories: pages that Auto flattens | ≤ 5% |
| Reference set, bent categories: character error rate against the corners alone | Lower in median, worse on no photo by more than 0.5 points |
| Book spreads, synthetic and reference set: spine within 1% of the spread's width | ≥ 95% |
| The tests listed under A1–A30 | All pass |
| Determinism: one thread and eight threads give byte-identical PDFs (Vision replaced by a recording) | Pass |
| Fuzzing: 10 000 malformed images and PDFs, and every command with junk parameters | No panic |
| UI snapshot tests (`egui_kittest`) of each screen in light and dark appearance | Pass |

For comparison, published neural detectors reach a mean Jaccard index of 0.985 (LDRNet) and 0.992 (HU-PageScan) on SmartDoc 2015 (as cited in pdfcraft-android #286).

---

## 11. Milestones

The milestones are checkpoints, not a sequence. `ScanCraft Build Plan.md` splits the work into 15 packages that agents build in parallel, after a first package fixes the workspace and the shared types, and says which packages each milestone needs.

| Milestone | Scope | Done when |
|---|---|---|
| M0 Workspace | Crates and layers (§6.1), lints, `cargo xtask ci`, licence checks | CI is green on macOS and Linux |
| M1 Core pipeline | A1–A7, A9–A14, A16–A19, A22–A24 in the command-line tool; synthetic scenes; `eval` | Synthetic gates pass; first SmartDoc and reference-set numbers; A9's constants tuned; HEIC speed measured |
| M2 Curved pages | A25–A30 in the command-line tool; bent synthetic scenes; the flattening error in `eval`; the six bent categories of the reference set | The synthetic flattening gates pass; first reference-set and UVDoc figures; A29's constants tuned |
| M3 macOS services | `scancraft-apple` with A8, A15 and A20; the A21 text layer; export options | Searchable PDFs; the full-pipeline SmartDoc gate passes |
| M4 Interface | Screens 1–3, loupe, edge and bend handles, Flatten and Book controls, spine handles, tray, menus, undo, autosave and restore | UI snapshot tests pass; the app works end to end |
| M5 Release | Accessibility, localization, settings, packaging (universal binary, Developer ID signing, notarized DMG) | Every gate in §10.4 passes |

---

## 12. Open decisions and risks

### 12.1 Decisions for the owner

1. **PdfCraft's PDF pieces: decided, ported** (15 November 2026; §5.3). Later, once ScanCraft's ported module stands alone, it can move into craft-libs so that PdfCraft shares one copy; that move needs PdfCraft's owner.
2. **Minimum macOS: decided, 13 Ventura** (15 November 2026). One Vision code path; WordCraft's 11 would have meant no Apple page detector on 11 and manual language choice on 12.
3. **Patent review** of the shadow-removal method before any commercial release; pdfcraft-android #298 flags a related patent.
4. **PDF/A-2b** (the archival PDF standard) as an export option. The writer would need XMP metadata and an sRGB output intent.
5. **App icon.** `scancraft.jpeg` has a white background and a baked-in shadow. Packaging needs a transparent 1024 px master and the layered icon format current macOS uses.
6. **A page detector beyond macOS.** If ScanCraft ever ships on other systems, A8 needs a portable counterpart. The candidate is DocQuadNet-256 (Apache-2.0, 6.7 MB as an fp16 ONNX file, run with rten), the neural detector pdfcraft-android adopted.
7. **A learned flattener for pages bent two ways.** UVDoc through rten could handle what A27 can't model, such as crumpled sheets and corners curled diagonally, behind A29's final check. It needs three things first: a licence for its trained weights, a check on the reference set that it leaves flat pages alone, and acceptance of 15.8 MB more in the app. pdfcraft-android's export script, conventions and tests (#302) can be reused.

### 12.2 Risks

| Risk | Mitigation |
|---|---|
| Pure-Rust HEIC decoding is too slow | Measured in M1; ImageIO fallback in `scancraft-apple` |
| Vision behaves differently after a macOS update | A9 checks Vision's corners against the same edge evidence as the classical candidates; evaluation reports record the macOS version |
| Even Light's thresholds were measured on only ten photos | The reference set covers 18 categories; the strength slider and the Original look remain available |
| `boundingBoxForRange:error:` has no safe binding | One audited `msg_send!` call; word boxes fall back to proportional splitting |
| A9's constants are tuned on a small set | They are re-tuned whenever the reference set grows, and the no-touch rate is a release gate |
| The Mac app has no OCR fallback | Vision ships with every supported macOS; a page that fails is reported, never silently left unsearchable |
| Text lines and edges in real photos are rougher than in the synthetic checks behind A27 | Cauchy weights, floors on the number of lines, and A29's final check; six bent categories in the reference set; Flatten Off always works |
| Without an EXIF focal length, the camera's distance is uncertain | The focal length is fitted with the sheet (A27); from a typical start, the synthetic error stayed at 0.09% for book pages |
| The edges of a book's underlying pages are traced instead of the top page's | With Book Spreads on, A25 searches inward (step 4); bend handles let the user correct a side |
| A photo is split into two pages when it shouldn't be | Splitting happens on its own only with Book Spreads on; otherwise Screen 2 only offers it, and Join Pages undoes it |

---

## 13. References

### Papers and standards

- Bayer, B. E. (1973). An optimum method for two-level rendition of continuous-tone pictures. *IEEE International Conference on Communications.*
- Bernsen, J. (1986). Dynamic thresholding of grey-level images. *Proc. 8th ICPR*, 1251–1255.
- Burie, J.-C. et al. (2015). ICDAR 2015 competition on smartphone document capture and OCR (SmartDoc). *ICDAR 2015.*
- Canny, J. (1986). A computational approach to edge detection. *IEEE TPAMI* 8(6), 679–698.
- Douglas, D., Peucker, T. (1973). Algorithms for the reduction of the number of points required to represent a digitized line or its caricature. *Cartographica* 10(2), 112–122.
- Duchon, C. E. (1979). Lanczos filtering in one and two dimensions. *Journal of Applied Meteorology* 18(8), 1016–1022.
- Duda, R., Hart, P. (1972). Use of the Hough transformation to detect lines and curves in pictures. *Communications of the ACM* 15(1), 11–15.
- Fischler, M., Bolles, R. (1981). Random sample consensus. *Communications of the ACM* 24(6), 381–395.
- Gil, J., Werman, M. (1993). Computing 2-D min, median, and max filters. *IEEE TPAMI* 15(5), 504–507.
- Graham, R. (1972). An efficient algorithm for determining the convex hull of a finite planar set. *Information Processing Letters* 1(4), 132–133.
- Heckbert, P. (1989). *Fundamentals of Texture Mapping and Image Warping.* MS thesis, UC Berkeley, UCB/CSD 89/516, §2.2.3.
- Higham, N. J. (1986). Computing the polar decomposition, with applications. *SIAM Journal on Scientific and Statistical Computing* 7(4), 1160–1174.
- Keys, R. (1981). Cubic convolution interpolation for digital image processing. *IEEE Trans. ASSP* 29(6), 1153–1160.
- Kim, B. S., Koo, H. I., Cho, N. I. (2015). Document dewarping via text-line based optimization. *Pattern Recognition* 48(11), 3600–3614.
- Knutsson, H., Westin, C.-F. (1993). Normalized and differential convolution. *CVPR 1993*, 515–523.
- Kovesi, P. (2010). Fast almost-Gaussian filtering. *DICTA 2010.*
- Levenberg, K. (1944). A method for the solution of certain non-linear problems in least squares. *Quarterly of Applied Mathematics* 2(2), 164–168.
- Liang, J., DeMenthon, D., Doermann, D. (2008). Geometric rectification of camera-captured document images. *IEEE TPAMI* 30(4), 591–605.
- Ma, K., Shu, Z., Bai, X., Wang, J., Samaras, D. (2018). DocUNet: document image unwarping via a stacked U-Net. *CVPR 2018.*
- Marquardt, D. W. (1963). An algorithm for least-squares estimation of nonlinear parameters. *Journal of the Society for Industrial and Applied Mathematics* 11(2), 431–441.
- Montanari, U. (1971). On the optimal detection of curves in noisy pictures. *Communications of the ACM* 14(5), 335–345.
- Nielsen, H. B. (1999). *Damping parameter in Marquardt's method.* Technical report IMM-REP-1999-05, Technical University of Denmark.
- Ottosson, B. (2020). A perceptual color space for image processing (Oklab). bottosson.github.io/posts/oklab.
- Paeth, A. (1990). Median finding on a 3 × 3 grid. *Graphics Gems.*
- Ramer, U. (1972). An iterative procedure for the polygonal approximation of plane curves. *Computer Graphics and Image Processing* 1(3), 244–256.
- Sauvola, J., Pietikäinen, M. (2000). Adaptive document image binarization. *Pattern Recognition* 33(2), 225–236.
- Suzuki, S., Abe, K. (1985). Topological structural analysis of digitized binary images by border following. *CVGIP* 30(1), 32–46.
- Triggs, B., McLauchlan, P., Hartley, R., Fitzgibbon, A. (2000). Bundle adjustment: a modern synthesis. *Vision Algorithms: Theory and Practice*, LNCS 1883, 298–372.
- Turkowski, K. (1990). Filters for common resampling tasks. *Graphics Gems.*
- van Herk, M. (1992). A fast algorithm for local minimum and maximum filters on rectangular and octagonal kernels. *Pattern Recognition Letters* 13(7), 517–521.
- Verhoeven, F., Magne, T., Sorkine-Hornung, O. (2023). UVDoc: neural grid-based document unwarping. *SIGGRAPH Asia 2023 Conference Papers.*
- Zhang, Z. (2000). A flexible new technique for camera calibration. *IEEE TPAMI* 22(11), 1330–1334.
- Zhang, Z., He, L. (2007). Whiteboard scanning and image enhancement. *Digital Signal Processing* 17(2), 414–432 (Microsoft Research MSR-TR-2003-39).
- Zucker, M. (2016). Page dewarping. mzucker.github.io/2016/08/15/page-dewarping; code at github.com/mzucker/page_dewarp (MIT).
- Zuiderveld, K. (1994). Contrast limited adaptive histogram equalization. *Graphics Gems IV.*
- ISO 32000-2:2020, *Document management — Portable document format — PDF 2.0*: §9.3.6 (text rendering mode), §7.4.7 (JBIG2Decode filter).
- ITU-T T.81 (JPEG); ITU-T T.88 (JBIG2); Unicode Standard Annex #29 (text segmentation); Unicode CLDR supplemental data (paper sizes by region).

### Code, issues and measurements

- craft-libs, github.com/storytold/craft-libs: `craft-image-codecs`, `craft-heif`, `craft-cms`, `craft-metadata`, `craft-jpeg`, `craft-stream-codecs`, `craft-pdf`, `craft-dialogs`, `craft-menu`, `craft-quit`, `craft-ui`, `craft-settings`, `craft-i18n`, `craft-atomic`.
- PdfCraft, github.com/storytold/pdfcraft (Apache-2.0), the source of the ported code: `crates/ocr` (`text_layer`, `detect_orientation`, and `OcrsRecognizer` as the model for the ocrs fallback), `crates/fonts` (`TextPlan`), `crates/edit` (`add_type3_fonts`) and `crates/create` (`from_images`); pull request #3033, which wraps the OCR layer in `q … Q`.
- VectorCraft, github.com/storytold/vectorcraft, pull request #3159: krilla has no invisible text mode.
- pdfcraft-android, github.com/MohamedRef/pdfcraft-android (MIT OR Apache-2.0): issues #286 (scanner tracking and the published detection figures), #288 (evaluation harness and the line-straightness measure), #290 (neural detector), #291 (edge refinement), #293 (true proportions), #298 (clean-scan look), #300 (black and white), #302 (flattening network and its guard, with the ten-photo measurements), #303 (book mode and the text-line fit); `docs/ANDROID_SCAN_FLATTEN.md`, `docs/ANDROID_SCAN_FLATTEN_LINES.md` and `docs/ANDROID_SCAN_BOOK.md`.
- UVDoc, github.com/tanguymagne/UVDoc (MIT; the trained weights carry no licence of their own), and the UVDoc benchmark, github.com/tanguymagne/UVDoc-Dataset.
- Apple, WWDC25 session 272, "Read documents using the Vision framework" (`RecognizeDocumentsRequest`), developer.apple.com/videos/play/wwdc2025/272.
- L-K-M/Kirsch, pull request #23: errors from sizing pages by their side lengths.
- imageproc 0.27.0, docs.rs/imageproc; objc2-vision 0.3.2, docs.rs/objc2-vision and its `translation-config.toml`.
- Apple Vision documentation: `VNDetectDocumentSegmentationRequest` (macOS 12 and later), `VNRecognizeTextRequest`.
