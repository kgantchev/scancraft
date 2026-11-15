# Reference photos

Real photos of documents, for the final quality checks (specification §10.2). They are school
worksheets and workbook pages, some with a name or handwriting on them, so they are not committed:
`.gitignore` excludes everything here except this file.

## Names

A photo is named for the **object** in it, and a number: `<object>-<nn>.jpg`.

| Object | Meaning | Now |
|---|---|---|
| `sheet` | a loose printed sheet, photographed flat | `sheet-01` to `sheet-06` |
| `book` | a page of a bound book or workbook, bent where it nears the spine | `book-01` to `book-07` |

Later objects use the same pattern: `spread`, `receipt`, `magazine`, `letter`, `handheld`. The name
says only what anyone can see. The labels that need judgement (the §10.2 category, the page's
corners, the ratio) belong in `reference.json`, where the loader reads them, so a label can change
without a file changing name. Numbers are never reused.

Files that belong to a photo share its stem:

- `sheet-01.naps2.pdf`: what another scanning app (NAPS2) made from that photo. It is a picture of
  the page with no text layer, so it is a **comparison**, not the truth. Photos `sheet-01` to
  `sheet-04` have one.
- `book-01.txt` (not here yet): the page's real text, for the character error rate. The loader
  reads `<name>.txt` through `text_file` in `reference.json`.
- `multipage-01.pdfcraft.pdf`: eight of the photos, byte for byte, as the pages of one PDF, in the
  order `book-01`, `book-07`, `book-06`, `sheet-06`, `book-05`, `book-04`, `book-03`, `book-02`. It
  is a multi-page input for the PDF importer. Its text layer is noise, so it is no reference for
  text.

## What is here

13 photos, JPEG, 1392–1623 × 1999 px (already scaled down from the camera's size) and 5 PDFs. There
is no HEIC and nothing at 12 megapixels, so the §10.2 HEIC tests and §9's timing budgets run on
synthetic scenes instead.

In most photos the page fills the frame or runs past it. That is the case A9's border rule is for,
and it is where the starting constants do worst.

## How the tests use them

- `scancraft-io` (`crates/io/tests/reference.rs`) imports, encodes and exports every photo and opens
  and renders every PDF. It skips, with a message, when the folder is missing. Other git worktrees
  (`../scancraft-wpN`) do not contain it: set `SCANCRAFT_REFERENCE_DIR` to this folder's path to run
  them there.
- `scancraft-cli eval --set reference` needs a `reference.json` manifest in this folder with each
  photo's `file`, `sha256`, `category` (one of the 18 of §10.2) and, to score corners, its `corners`
  (TL, TR, BR, BL, in photo pixels). A photo whose bytes no longer hash to `sha256` is skipped.
  The format is in `crates/synth/README.md`. There is no manifest yet.

The release gates this set feeds (§10.4) need at least 90 labelled photos in the 18 categories, and
real text for the bent ones. These 13 are a start.
