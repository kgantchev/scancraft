# ScanCraft Build Plan

**Companion to the ScanCraft Application Specification, version 2.0** · 15 November 2026

The app is split into 15 work packages that agents can build in parallel. One package, WP0, runs first and alone. It fixes the workspace, the shared types and every function signature. After that, up to seven agents work at once, each in its own crate or module, against those fixed signatures. The specification's milestones M0–M5 (§11) are checkpoints: each is reached when the packages it needs have merged.

## 1. Schedule

```text
Wave 0   WP0 Foundations ────────────────────────────────────────────────┐ (one agent)
                                                                         │
Wave 1   WP1 Geometry ───────────┐                                       │ (seven agents)
         WP2 Raster ─────────────┼──┐                                    │
         WP3 Import and export   │  │                                    │
         WP4 Synthetic scenes and evaluation                             │
         WP5 Apple services      │  │                                    │
         WP6 Engine              │  │                                    │
         WP7 Interface           │  │                                    │
                                 │  │                                    │
Wave 2   WP8 Page detection ◄────┼──┤ (needs WP2)                        │ (three agents)
         WP9 Enhancement ◄───────┼──┤ (needs WP2)
         WP10 Text lines and spreads ◄┘ (needs WP2)
                                 │
Wave 3   WP11 Curved-page integration ◄ WP1, WP6, WP8, WP10              │ (three agents)
         WP12 Pipeline integration and gates ◄ WP1–WP4, WP8, WP9, WP11 (finishes after WP11)
         WP13 App integration ◄ WP5, WP6, WP7, WP12
                                 │
Wave 4   WP14 Release ◄ everything                                       (one or two agents)
```

| Wave | Packages | Agents at once | Starts when |
|---|---|---|---|
| 0 | WP0 | 1 | Now, once the two decisions in §5 are made |
| 1 | WP1–WP7 | 7 | WP0 is merged |
| 2 | WP8–WP10 | 3, while wave 1 finishes | WP2 is merged; they don't wait for the rest of wave 1 |
| 3 | WP11–WP13 | 3 | Each package's own inputs are merged |
| 4 | WP14 | 1–2 | WP13 is merged |

Wave 2 overlaps wave 1, so the peak is ten agents at once, if WP2 finishes early.

Milestones, as checkpoints:

| Milestone | Reached when merged |
|---|---|
| M0 Workspace | WP0 |
| M1 Core pipeline | WP1, WP2, WP3, WP4, WP8, WP9, and WP12's flat-page gates |
| M2 Curved pages | WP10, WP11, and WP12's flattening gates |
| M3 macOS services | WP5 and WP13's searchable-PDF check |
| M4 Interface | WP6, WP7, WP13 |
| M5 Release | WP14 |

## 2. Rules for every agent

1. **One package, one branch, one worktree.** Work in `git worktree add ../scancraft-wpN -b wpN`. Never edit another package's files.
2. **Own only what the table in §4 lists.** If a shared type or signature in `scancraft-core` must change, stop and open a small separate pull request titled `contract: …`, which the integrator (§6) merges first. Don't change contracts inside a package's branch.
3. **No new workspace members or dependencies.** WP0 declares every crate and every dependency in the workspace `Cargo.toml`. A package that needs another dependency asks for it through a contract pull request.
4. **Stubs stay callable.** WP0 creates every public function from the specification as a stub returning `Err(Error::NotImplemented("A16"))`. A package replaces the bodies it owns and keeps the signatures. Code that calls an unfinished step compiles and gets an error.
5. **Done means the specification's tests pass.** Every algorithm in a package has the **Tests** listed under it in the specification §8, written as Rust tests, plus `cargo clippy` with the Craft lints, plus a one-thread against eight-thread determinism test for each pixel loop.
6. **Never crash** (specification §6.8): no `unwrap`, `expect`, `panic!` or indexing without `get` outside tests, and `unsafe` only in WP5's listed call sites.
7. **Test data.** Until WP4's generator lands, use the small fixtures WP0 commits in `fixtures/`. After that, generate scenes with `scancraft-cli synth` and a fixed seed; don't commit large generated files.
8. **Report** in the pull request: which specification tests pass, measured times against §9's budgets, and anything left undone. Don't claim figures that weren't measured.

## 3. WP0 Foundations (wave 0, one agent)

WP0 runs alone, because every other package builds on what it fixes. Keep it to skeletons and contracts; it implements only the few small pieces many packages need.

1. **Repository.** `git init` in `scancraft/`, the workspace of specification §6.1 with every crate created and empty: `scancraft-core`, `scancraft-geom`, `scancraft-raster`, `scancraft-detect`, `scancraft-enhance`, `scancraft-io`, `scancraft-engine`, `scancraft-apple`, `scancraft-ui-egui`, `scancraft-synth`, and the apps `scancraft` and `scancraft-cli`. `scancraft-core` is a new L0 crate that holds only the shared types below, so no package has to own them.
2. **Rules files.** `CLAUDE.md` and `AGENTS.md`, adapted from WordCraft's: the Craft lints, never-crash, "Rust only, no handwritten JS/TS", the one `unsafe` exception (§6.7), and §2 of this plan.
3. **Dependencies.** Every crate's dependencies from specification §5.7, pinned: craft-libs at a fixed git revision, `pdf-writer` 0.15, egui 0.37 from the `storytold/egui` fork at WordCraft's revision. `cargo xtask layers` enforces §6.1's layering; `cargo xtask ci` runs the format check, clippy, tests and the layer check, and `cargo xtask ci -p <package>` does the same for one package.
4. **Shared types in `scancraft-core`:**
   - geometry: `Point`, `Quad` with the canonical corner order of §7.2, `Side`, `Line`;
   - pixel buffers: `Rgb8`, `Gray8`, `Bit1`, `PlaneF32`, with the fixed 64-row band iterator of §7.6;
   - determinism: `Xorshift64Star`, the stable hash for seeds, `ALGO_VERSION`;
   - the page recipe of §6.2 with every field type (`SideShape`, `SpreadHalf`, `FlattenMode`, `SheetFit`, `Proportions`, `Look`, …), all `serde` with a versioned session format;
   - the camera of §7.8 and the luma pyramid type A10 and A25 share;
   - error types per crate, including `NotImplemented`;
   - the traits `PageDetector` (A8), `UprightVoter` (A15) and `Recognizer` (A20), modelled on PdfCraft's;
   - the command list of §6.3 as an enum with parameters, and the `EngineApi` trait the interface talks to, with view-model types for each screen.
5. **Stubs.** Every algorithm A1–A30 as a public function with its final signature, in the crate §6.1 assigns, returning `NotImplemented`.
6. **Small real pieces:** A12 homography (about 40 lines, used by six packages), the `Xorshift64Star` generator, the band iterator, and A3 area downscale. Each with its specification tests.
7. **Fixtures.** `fixtures/`: three drawn scenes with exact truth files (a flat page, a book page curling into its spine, a two-page spread), in the scene format WP4 also writes. `cargo xtask fixtures` redraws them. Real photos go into the reference set, outside the repository.
8. **CI.** The suite's shared gate (`storytold/craft-common-actions`, house rule 11) runs `cargo xtask ci` on Linux and macOS. A failing package shows in the gate's per-crate test output; packages check themselves with `cargo xtask ci -p` before their pull requests.

Done when `cargo xtask ci` is green on macOS and Linux and every stub compiles.

## 4. Work packages

| Package | Wave | Owns | Specification sections | Needs | Delivers |
|---|---|---|---|---|---|
| WP0 Foundations | 0 | workspace, `scancraft-core`, `xtask`, CI, `fixtures/`, A3, A12 | §5.7, §6, §7, A3, A12 | §5 decisions | M0 |
| WP1 Geometry | 1 | `scancraft-geom` except A12 | A11, A27, A28; §7.8 | WP0 | Proportions, the spline, the Levenberg–Marquardt solver, the sheet fit and unrolling, tested on synthetic points as in A27's prototype |
| WP2 Raster | 1 | `scancraft-raster` except A3 | A4, A13, A14, A19, A18 steps 1–5 as a helper, colour and Oklab | WP0 | Blur, warp (through a homography or any mapping), resampling, min/max filters, the ink rule |
| WP3 Import and export | 1 | `scancraft-io` | A1, A2, A21, A22, A23, A24; §6.6 file writing | WP0 | Decode, colour conversion, PDF import, encoding, the text layer and PDF assembly on `pdf-writer`. The text layer, word fitting, Type3 fonts and image pages are ported from PdfCraft at a recorded commit, with its tests and the Apache-2.0 notice (specification §5.3); tested with fixed images and word lists |
| WP4 Synthetic scenes and evaluation | 1 | `scancraft-synth`, the `synth` and `eval` subcommands of `scancraft-cli` | §10.1–§10.3 | WP0 | The scene generator, flat and bent, with its own independent camera and sheet code; dataset loaders for SmartDoc 2015, UVDoc and the reference set; every metric of §10.3 |
| WP5 Apple services | 1 | `scancraft-apple` | A8, A15 (Vision voter), A20; §6.7 | WP0; a Mac | The three Vision calls behind their traits, with recorded Vision results for CI |
| WP6 Engine | 1 | `scancraft-engine` except A29 | §6.2–§6.6 | WP0 | Recipes, commands, undo, the job runner, caches, sessions, restore, running on stub pipeline steps |
| WP7 Interface | 1 | `scancraft-ui-egui`, the `scancraft` app shell | §4 | WP0 | Screens 1–3, canvas, loupe, corner, edge and bend handles, spine, tray, menus, settings, against a `FakeEngine`, with snapshot tests |
| WP8 Page detection | 2 | `scancraft-detect`: A5–A7, A9, A10 and the pyramid | A5–A7, A9, A10 | WP2 | Corners from a photo, classical path, with A8 through the trait |
| WP9 Enhancement | 2 | `scancraft-enhance` | A15 (fallback), A16, A17, A18 | WP2 | Even Light, the looks, Black & White |
| WP10 Text lines and spreads | 2 | `scancraft-detect`: the `text_lines` and `spread` modules | A26, A30 | WP2 | Text-line evidence and the spread split, tested on WP0's fixtures and drawn pages |
| WP11 Curved-page integration | 3 | `scancraft-detect`: the `curved_sides` module and A9's change; `scancraft-engine`: the `flatten` module | A25, A29, A9 step 6.2 | WP1, WP6, WP8, WP10 | Curved sides, the flatten decision, and flattening wired into the engine |
| WP12 Pipeline integration and gates | 3 | the `page`, `pdf`, `detect` and `flatten` subcommands of `scancraft-cli`; tuning notes | §6.9, §9, §10.4 | WP1–WP4, WP8, WP9; WP11 for the curved gates | The whole pipeline in the command-line tool; A9's and A29's constants tuned; the release gates run and reported |
| WP13 App integration | 3 | the `scancraft` app's wiring, swapping `FakeEngine` for the real engine | §3, §6.4 | WP5, WP6, WP7, WP12 | The Mac app working end to end with Vision |
| WP14 Release | 4 | packaging, `craft-i18n` strings, accessibility fixes, fuzz targets | §4.9, §6.10, §9, §10.4 | everything | Every gate of §10.4; a notarized DMG |

WP8, WP10 and WP11 all work inside `scancraft-detect`, in separate modules that WP0 creates, so their branches don't touch the same files.

WP12 starts once the flat-page packages merge, and adds the curved gates when WP11 lands; it can begin as soon as WP8 and WP9 are in.

## 5. Decisions needed before WP0

1. **PdfCraft's PDF pieces: decided, ported** into `scancraft-io` on `pdf-writer` (specification §5.3). WP3 does it.
2. **Minimum macOS version: decided, 13.** It sets WP5's code paths and WP14's packaging.

The other open decisions (§12.1 items 3–7) don't block any package. The app icon (§12.1 item 5) is needed only by WP14, and `scancraft.jpeg` is no longer in the folder.

## 6. Running it

1. **The integrator.** One person or one long-lived agent merges pull requests, merges `contract:` pull requests first, keeps `Cargo.lock` consistent, and reruns `cargo xtask ci` on `main` after each merge. Packages rebase on `main` daily.
2. **Launching a package.** Start a fresh agent in the package's worktree on the Anthropic backend, with this prompt filled in:

   ```text
   You are building work package WPn (<name>) of ScanCraft.
   Read: CLAUDE.md, AGENTS.md, "ScanCraft Build Plan.md" §2 and the WPn row of §4,
   and in "ScanCraft Application Specification.md" the sections listed for WPn.
   Own only the files that row lists. Replace the stubs for your algorithms with
   implementations that pass every test the specification lists under them.
   Don't change shared types or signatures; if one must change, stop and describe
   the change. Finish with a pull request that reports which tests pass, measured
   times against §9, and anything not done.
   ```

3. **Sizing.** A package large enough to fill an agent's context, such as WP7 or WP8, may go to two agents in sequence, splitting at a natural line: Screens 1 and 2 before Screen 3 and the tray, or A5–A7 before A9 and A10. A second agent picks up from the first one's pull request report.
4. **When a package is blocked** by a missing piece of another package, it writes a test fixture or a fake for that piece and goes on. It never implements the other package's algorithm itself.
