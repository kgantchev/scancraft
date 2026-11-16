# Attribution

Every asset in this repository, with its author, source and licence.

| File | Author | Source | Licence |
|---|---|---|---|
| `fixtures/flat.png`, `fixtures/curved.png`, `fixtures/spread.png` and their `.json` truth files | ScanCraft contributors | Drawn by `cargo xtask fixtures` | MIT OR Apache-2.0 |
| `crates/io/src/helvetica.rs` (the standard Helvetica advance widths, data from the published Core 14 metrics) | ArtCraft Team and the PdfCraft contributors | PdfCraft `crates/fonts/src/std14_metrics.rs`, commit c944f106dd63eb9a33c7348eed1f03548cbd6468 | MIT OR Apache-2.0 |
| `apps/scancraft/assets/scancraft.svg`, `scancraft-1024.png`, `scancraft-256.png`, `ScanCraft.icns` (the app icon) | ScanCraft contributors | Vectorised from the project's `scancraft.jpeg` (specification §12.1 item 5); the PNGs and the `.icns` are rendered from the SVG | MIT OR Apache-2.0 |

## Datasets the tests read

These are not in the repository. A test reads one when its folder is named in an environment
variable, and skips with a message when it isn't. Nothing from them is copied into the repository,
and no result file made from them is committed.

| Dataset | Authors | Source | Licence | Used by |
|---|---|---|---|---|
| SmartDoc 2015, challenge 1 (version 2.0.0 of the easy-to-parse mirror): about 25 000 frames of 30 printed documents on five backgrounds, with the true corners of the page in each frame | Jean-Christophe Burie, Joseph Chazalon, Mickaël Coustaty, Sébastien Eskenazi, Muhammad Muzzamil Luqman, Maroua Mehri, Nibal Nayef, Jean-Marc Ogier, Sophea Prum and Marçal Rusiñol; the mirror's format by Joseph Chazalon | <https://github.com/jchazalon/smartdoc15-ch1-dataset> (release v2.0.0); original at <http://smartdoc.univ-lr.fr/> | CC-BY-4.0 | The detection gate of specification §10.4 (`crates/detect/tests/smartdoc.rs`, `crates/apple/tests/smartdoc_vision.rs`; `SCANCRAFT_SMARTDOC_DIR`) |

The licence asks that the work be credited by citing: Burie, J.-C., Chazalon, J., Coustaty, M.,
Eskenazi, S., Luqman, M. M., Mehri, M., Nayef, N., Ogier, J.-M., Prum, S. and Rusiñol, M.,
"ICDAR2015 Competition on Smartphone Document Capture and OCR (SmartDoc)", 13th International
Conference on Document Analysis and Recognition (ICDAR), 2015. The original images remain the
property of their authors.
