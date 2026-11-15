# Reference photos

Real photos of documents, for the final quality checks (specification §10.2). They show people's
handwriting, so they are not committed: `.gitignore` excludes everything here except this file.

- 14 JPEG photos, about 1400–2000 px on the long side: `eng-hw-01` to `eng-hw-08`, `bobo-hw-01`,
  `bobo-hw-02`, `bobo-hw-04`, `bobo-h2-03`, `bg-hw-01` and `1000025935`.
- 4 PDFs that share a name with a photo (`bobo-hw-01`, `bobo-hw-02`, `bobo-hw-04`, `bobo-h2-03`).

The `scancraft-io` integration tests (`crates/io/tests/reference.rs`) import, encode and export every
photo here and open every PDF; they skip with a message when the folder is missing. Other git
worktrees (`../scancraft-wpN`) do not contain it: set `SCANCRAFT_REFERENCE_DIR` to this folder's
path to run them there.

These photos are not yet a reference set for `scancraft-cli eval --set reference`, which needs a
`reference.json` manifest with each photo's corners, category and SHA-256
(`crates/synth/README.md`).
