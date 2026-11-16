# WP12 tuning notes: the pipeline in the command-line tool and the release gates

What was run, on which data, and what it showed. Every figure below was measured on this build
(release profile, synthetic scenes from `scancraft-cli synth`). The real data sets were not
available, so the gates that need them were not run.

## What the tool does now

- `detect <photo>... [--book] [--order ltr|rtl] [--json]`: finds the page. One record per photo,
  in the shape `eval --found` reads (`id`, `corners`, `size`), plus the origin, the sides, the spine
  and the time A1–A10 and A25 took (`detect_ms`).
- `flatten <photo>... [--flatten auto|on|off] [--json]`: the same, then applies the crop and waits
  for A29. Adds `flattened`, and per page the whole decision record (reason, D95, both costs, the
  straightness figures).
- `page <photo> --out page.png [--look] [--strength] [--flatten] [--threads]`: the page at its
  output size.
- `pdf <photo>... --out scans.pdf [--look] [--strength] [--flatten] [--book] [--order] [--quality]
  [--threads]`: one PDF, pages in the order of the command line.
- Each runs the real engine by commands, in a session folder made for the run and removed after.
  Nothing opens a dialog.
- `--ocr auto` is accepted and says that this build carries no text-recognition models, so the PDF
  has no text layer. The ocrs adapter and its models are not part of this package.

## Findings in other packages (not changed here)

1. **Screen 2 opens on the photo whose detection finished first**, not the first one imported
   (`work.rs`, `apply_detected`). With several photos, which page is first in the tray therefore
   depends on timing and thread count: the same command line wrote PDFs with the pages swapped on
   one thread and on eight. The tool avoids it by opening the first photo explicitly. The app has the
   same problem; it belongs to WP6 or WP13.
2. **A30 is still not wired** (known): `--book` cuts each photo down the middle.

## The gates

| Gate (§10.4) | Target | Measured | Data |
|---|---|---|---|
| Determinism: one thread and eight give byte-identical PDFs | pass | **pass** (`tests/cli.rs`, three photos, also over three repeated runs each) | fixtures |
| Fuzzing: every command with junk parameters | no panic | **pass for the four subcommands** (`tests/cli.rs`: bad flags exit 64, junk, truncated and missing files exit 1). The 10 000-file fuzz run belongs to WP14. | fixtures |
| Synthetic flat pages that Auto flattens | ≤ 2% | **5.0%** (2 of 40), 4.2% on a second run of 24 | synth seed 9001 |
| Synthetic bent pages, flattening error, median | ≤ 0.25% | **0.89%** as Auto leaves them, 0.72% if every page is flattened, 2.14% from the corners alone | synth seed 9002, 40 scenes, true outline given |
| The same, without an EXIF focal length | ≤ 0.35% | the same figures: synthetic photos are PNG with no focal length, so these are the "without" figures. The "with" figure was not measured | |
| Classical detection on synthetic scenes (no gate; SmartDoc's is ≥ 0.93 Jaccard) | | no-touch 17 of 60 | synth seed 7 |
| SmartDoc 2015, reference set, UVDoc, spine within 1%, character error rate, line straightness on reference photos, UI snapshots | | **not run**: no data on this machine (SmartDoc, UVDoc, the reference set), A30 unwired, no recogniser in the tool | |

Reproduce with:

```text
cargo test --release -p scancraft-cli --test gates -- --ignored --nocapture      # SCANCRAFT_GATE_COUNT=40
cargo test --release -p scancraft-detect --test gates -- --ignored --nocapture   # SCANCRAFT_GATE_COUNT=60
```

## Detection on the hard backgrounds

The classical path (A5–A7, A9, A10) with no Vision, on 60 scenes of seed 7, by background:

| Background | Scenes | A candidate within 2% exists | Found to within 0.5% | Not found |
|---|---|---|---|---|
| Dark desk | 9 | 8 | 7 | 0 |
| Wood | 12 | 8 | 6 | 2 |
| Light desk | 12 | 5 | 4 | 2 |
| Checked cloth | 14 | 0 | 0 | 0 |
| White on white | 13 | 0 | 0 | 6 |
| All | 60 | 21 | 17 | 10 |

The shortfall is in candidate generation (A5–A7), not in A9's scoring: on the cloth and the white
surface no candidate comes near the page, so no weight in A9 could pick it. The cloth's checks
outrank the page's edge, and on white-on-white the edge is a faint shadow, often with a side outside
the frame. These are the scenes Vision (A8) is there for; the full-pipeline gate on macOS is
WP13's. An experiment raising A7's line limit from 16 to 40 gave 19 instead of 17 of 60 and was not
kept: with no real photos to check it against, it would be tuning to the synthetic set.

**A9's constants were therefore left as the specification gives them.** The classical SmartDoc gate
(≥ 0.93) needs the data set and is open.

## Flattening: where the error comes from (40 bent scenes, true outline given)

The fits are bimodal. Where A27 fits well the page comes out within about 0.1–0.3% (sheets curled
through 22–58°, folded letters, book pages up to 35°); where it does not, the error is 2–15%.

- **Auto against always-on.** Check 4 (straighter lines) leaves a page unflattened in 6 of 40
  scenes where flattening was right: folded letters #1 and #26 (2.8% and 3.9% unflattened, 0.15%
  and 0.26% flattened) and a sag, #25 (3.5%, 0.72%). It was right in 5 others, where flattening was
  worse (#10, #15, #18, #36, #37). Relaxing it would trade these about evenly on this set, so it
  stays at the specification's value until real photos say otherwise.
- **Hard fits.** Sheets curled through 56–79° and the steepest book pages (59°) fit badly
  (5–15%), flattened or not. This is the model's limit, not a constant: A27 is where to look.
- **Open books ("open book with a gutter")** come out about the same with and without flattening
  (0.3–0.8%, 5.7% in one): the spread is not split at its spine (A30 unwired), so A27 fits one sheet
  to two.
- **Flat pages flattened (5%).** The two cases were not examined further.

## What is left

- Gates that need data: SmartDoc, the reference set, UVDoc.
- A30 wiring, which has no owner, and the book gates behind it.
- The ocrs adapter for the tool.
- Tuning A27's fit for steep curls and book pages, and A29's constants against real photos (M2).
