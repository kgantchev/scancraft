# ScanCraft

A macOS app, in Rust, that turns photos of paper documents into clean, searchable PDFs: it finds
each page's edges, flattens the page (perspective, and curl when the page is bent), evens out its
light, reads its text and writes one PDF.

- What it does and how, and how it is being built in parallel work packages: the ScanCraft Application Specification and Build Plan, which are kept outside this repository, in the project notes
- Rules for contributors and agents: [`AGENTS.md`](AGENTS.md)

```text
cargo xtask ci                 # the full gate
cargo xtask ci -p scancraft-geom
cargo run -p scancraft         # the app (screens arrive with WP7)
cargo run -p scancraft-cli -- commands
```

Licensed under MIT OR Apache-2.0.
