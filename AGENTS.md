<!-- craft-house-rules:start core source=dde09ea31e2e457f5b4653a6e76f7cdf55944bdf sha256=540e6cea75d12d0710c3dca89658328e81e508ecef102446ca5bb95b7c0b92bb -->
## House rules

These apply in every Craft repository. A later rule may be stricter, not weaker. The source is `standards/house-rules-core.md` in `storytold/craft-contrib`. Propose changes there, not in a copy.

1. **A person is accountable.** Whoever runs an agent answers for what it opens and for its review comments.
2. **Check, then claim.** Before writing code, read the issue comments, search pull requests for `#N` and the keywords, and check recent commits on `main`. Comment on the issue first. Release a claim you will not finish.
3. **Verify, don't guess.** Open a definition before using it. Confirm the cause in the code, even when someone names a location.
4. **Keep fixes narrow and tested.** Fix what the issue describes, where the bug is. Add a test that fails without the fix.
5. **Review the diff before opening a pull request.** Identifiers exist as used, the test fails without the fix, nothing unrelated changed, and the description is true.
6. **Look for a better pull request.** If another fixes the same thing, close yours when it is better or equal, and say why.
7. **Report only what ran.** Say what you tested and what you could not run. Without a local build or test, say so and rely on CI before marking ready.
8. **`main` changes through a pull request.** Do not push to it. Branch protection enforces this. There is no exception.
9. **Stop and ask only about** licensing outside the allowlist, publishing, releases, a new remote, secrets, spending money, destructive git, and anything else hard to undo. Otherwise keep working. `plan/` is a gitignored notebook, not rules. Without it, do not ask for it and do not stop.
10. **Keep secrets and personal details out** of the repository, logs and requests.
11. **CI stays shared.** Workflows call `storytold/craft-common-actions`. Do not change CI in a feature or fix pull request. If CI cannot cover a change, open an issue there.
12. **Guards say why.** A check that looks removable says what it prevents. Read that before changing it.
13. **Instructions must be readable by everyone.** No notes about one machine, and no links to files a reader cannot open. `craftrules` is retired. Do not link to it.
14. **Keep pull requests moving.** Concurrent pull requests are fine; unanswered actionable feedback is not. Respond in 7 days, a reminder at 14, closure eligible at 30, from when an actionable request waits on the author. Suggestions and unconfirmed bot findings do not start the clock. A fix, explanation or agreed plan resets it; unrelated activity does not. Waiting on a reviewer or maintainer pauses it and does not count against the author. A maintainer decides closure; it is never automatic. Reuse search results, do not poll, and stop at a rate limit.
15. **No agent attribution.** A pull request does not say an agent wrote it. A repository rule may not require that.
<!-- craft-house-rules:end -->
<!-- craft-house-rules:start rust source=dde09ea31e2e457f5b4653a6e76f7cdf55944bdf sha256=387e0ccb4b49e567f60951d7d6fff7305054c89f43ad812b9993dc7c38528013 -->
## Rust Craft repositories

These apply to native Craft apps and to shared Rust libraries. Select them by repository type. A repository that merely contains some Rust does not take this block. Rules later in a repository's file may be stricter, but not weaker. The source is `standards/house-rules-rust.md` in [storytold/craft-contrib](https://github.com/storytold/craft-contrib).

1. **Never crash.** Non-test code does not panic and has no `unsafe`, except where the repository file names an exception. Return errors, treat input as hostile, and bound recursion and allocations. New production crate roots copy the `deny` attribute from an existing crate: `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::unimplemented`, `clippy::todo`, `clippy::unreachable`. Every crash fix comes with a test that panicked before the fix.
2. **Clean room.** Learn from observed behaviour and public specifications. Do not copy another product's code, assets, strings or artwork. Check a dependency's licence before adding it.
3. **Assets and fonts.** Every asset is original or suitably licensed, and is recorded in `ATTRIBUTION.md` in the same change. Font files live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), not in the app repository.
4. **Rust only for native Craft apps.** No handwritten JavaScript or TypeScript, and no Tauri, Electron or webview shell, in those apps. Shared tooling follows its documented technical scope and maintainer-approved exceptions. Craft PR Review's existing Tauri and TypeScript UI is one such exception. It does not relax this rule for native apps. Preserve any existing web build.
5. **Time goes through `chrono`.** Do not hand-roll calendar conversion. For dates beyond what `chrono` can represent, follow [standards/time.md](https://github.com/storytold/craft-contrib/blob/main/standards/time.md).
6. **Contributor credits.** Add or edit only your own person's entry in `contributors/people.toml`, and only after asking them. Never guess a name or hand-edit `contributors.json`.

## Licence allowlist

Dependencies and assets use SPDX identifiers. An expression is allowed when every identifier is in this list. A repository may name a shorter list. It may not add an identifier here by editing its own copy of these rules.

- `ISC`
- `MIT`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `Apache-2.0`
- `OFL-1.1`
- `Ubuntu-font-1.0`
- `Bitstream-Vera`
- `CC0-1.0`
- `CC-BY-4.0`
- `CC-BY-SA-4.0`
- `LicenseRef-Public-Domain`

`LicenseRef-Storyteller-Trademark` is not in this list. It covers Storyteller brand marks only, where a repository already says so.
<!-- craft-house-rules:end -->

# ScanCraft — instructions for agents

ScanCraft is a macOS app, in Rust, that turns photos of paper documents into clean, searchable PDFs. It finds each page's edges, flattens it (perspective, and curl when the page is bent), evens out its light, reads its text and writes one PDF. The command-line tool runs the same engine on Linux.

## Start every session here
- The ScanCraft Application Specification (kept outside this repository, in the project notes) says what to build and how, algorithm by algorithm (A1–A30), with each algorithm's tests.
- The ScanCraft Build Plan (kept with it) splits the work into packages WP0–WP14 and says which files each one owns. Work only on your package's files (plan §2).

## Never crash
- No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!` outside tests. Production crate roots carry `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`; new crates start with it. Tests may `unwrap` (`clippy.toml`).
- `unsafe_code = "forbid"` everywhere, with **one named exception**: `crates/apple` sets `unsafe_code = "deny"` and allows `unsafe` only at the audited Objective-C call sites of specification §6.7 (`boundingBoxForRange:error:` through `objc2::msg_send!`, and any accessor the bindings generate as `unsafe`), each with a `// SAFETY:` comment.
- Errors are `Result` values through each crate's error type. Every number derived from input is hostile: `get()` instead of indexing, checked arithmetic, capped allocations (`scancraft_core::image` limits), bounded loops.
- An unfinished step returns `CoreError::NotImplemented(Step::An)` (`scancraft_core::not_implemented`), never a panic.

## Architecture
| Layer | Crate | Job |
|---|---|---|
| L0 | `core` | shared types only: geometry, pixel buffers, the page recipe, commands, service traits, `EngineApi` |
| L0 | `geom` | homography (A12), proportions (A11), the sheet fit and unrolling (A27, A28) |
| L1 | `raster` | downscale (A3), blur (A4), warp (A13), resample (A14), helpers (A19), the ink rule (A18 steps 1–5) |
| L2 | `detect` | page detection (A5–A7, A9, A10), curved sides (A25), text lines (A26), spreads (A30) |
| L2 | `enhance` | upright (A15), Even Light (A16), looks (A17, A18) |
| L2 | `io` | import (A1, A2, A24), encoding (A22), text layer and PDF (A21, A23) |
| L3 | `engine` | sessions, recipes, commands, undo, jobs, caches, the flatten decision (A29) |
| L3 | `apple` | Vision services (A8, A15, A20); macOS only; only the apps use it |
| L4 | `ui-egui` | the interface |
| tool | `synth` | synthetic scenes and metrics; uses only `core`, so it checks the pipeline independently |
| apps | `apps/scancraft`, `apps/scancraft-cli` | the Mac app; the command-line tool |

`cargo xtask layers` enforces this. **`crates/core` is a contract**: change it only in a pull request titled `contract: …` that does nothing else (build plan §2).

**Dependencies**: the workspace `Cargo.toml` declares every crate and every dependency. A package may add to its own crate's `Cargo.toml` any dependency that manifest already declares, written as `{ name } = { workspace = true }` — a declared dependency the package had not needed yet, not a new one. It may not introduce a dependency the workspace does not declare; that needs a `contract:` pull request (build plan §2).

- **Everything is a command** (`scancraft_core::command::Command`). Buttons, menus, shortcuts, the CLI and tests run commands by id. Commands run by a program never open dialogs.
- **Deterministic:** pixel loops work in fixed 64-row bands (`scancraft_core::image::bands`); random sampling uses `Xorshift64Star` seeded by `seed_for`. One thread and eight threads must give identical results; test it.
- **Rust only** (no handwritten JS/TS). PDFs are written with `pdf-writer`; code ported from PdfCraft (Apache-2.0) records its source commit and keeps PdfCraft's notice (`NOTICE`).

## Quality gates
- Whole workspace: `cargo xtask ci` (fmt check, `clippy -D warnings`, tests, layers).
- One package before its pull request: `cargo xtask ci -p <package>`.
- Fixtures: `cargo xtask fixtures` redraws `fixtures/` (three drawn scenes with exact truth files). Don't commit large generated scene sets; generate them with `scancraft-cli synth` and a seed.

## Parallel agents
- One package, one branch, one worktree: `git worktree add ../scancraft-wpN -b wpN`.
- A separate `CARGO_TARGET_DIR` per agent; delete it when done (disk).
- Only the integrator merges and resolves `Cargo.lock`.

## CI workflows
Workflows in `.github/workflows/` call the shared workflows in `storytold/craft-common-actions` at `@main` (house rule 11). Don't change CI in a feature or fix pull request.
