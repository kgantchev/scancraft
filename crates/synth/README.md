# scancraft-synth

Synthetic scenes and the evaluation of specification §10. It uses `scancraft-core` and nothing else
from the workspace, so what it draws and measures is independent of the pipeline it checks.

## Synthetic scenes (`scancraft-cli synth`)

```text
scancraft-cli synth [--curved] --count N --seed S --out DIR
```

A scene is four files, `scene_00000.*`:

| File | Holds |
|---|---|
| `.png` | the photo, 8-bit sRGB |
| `.json` | a `scancraft_core::scene::SceneTruth`: corners (TL, TR, BR, BL), ratio, focal length, shape, spine, and for bent pages the 17 × 17 grid of where each page point lies |
| `.ink.png` | the page's true ink at 300 DPI in the flat page's own pixels, 8-bit grey, 255 = ink |
| `.txt` | the page's words, a line each, for the character error rate |

The same seed gives the same bytes on one build, on one thread or eight. Every purpose draws from its
own random stream, so adding a draw in one place never shifts another.

`profile_turning` holds the profile's **heading** at 33 evenly spaced arclengths, as the fixtures do
(the field is named for the turning angle).

## Data sets (`scancraft-cli eval --set NAME --root DIR`)

| Set | Layout read |
|---|---|
| `smartdoc2015` | `frames/metadata.csv.gz` (or a plain `metadata.csv`); the corners are tl, bl, br, tr in the file and TL, TR, BR, BL in memory |
| `uvdoc` | `grid2d/<name>.{mat,npy,json}` and `img/<name>.png` under `UVDoc_benchmark/`; the released `.mat` files are HDF5, which nothing in the workspace reads: convert once with `np.save(name + ".npy", hdf5storage.loadmat(path)["grid2d"])` |
| `reference` | a folder of photos and a `reference.json` manifest (below); a photo whose bytes no longer hash to its `sha256` is skipped |
| `synthetic` | a folder written by `synth` |

```json
{ "photos": [ {
    "file": "IMG_0412.HEIC", "sha256": "…", "category": "dark desk",
    "corners": [[x, y], [x, y], [x, y], [x, y]], "ratio": 0.7071,
    "text_file": "IMG_0412.txt", "spine": [[x, y], [x, y]], "order": "ltr"
} ] }
```

The categories are the 18 of §10.2; a book spread is one category, in either reading order.

## The pipeline's results (`--found`)

`eval` scores what the pipeline found, given as `--found results.json`: a JSON array (or
`{"photos": [...]}`) with one record per photo.

```json
{ "id": "scene_00000", "corners": [[x, y], [x, y], [x, y], [x, y]] , "ratio": 0.7071,
  "grid": [[[x, y], …], …], "text": "…", "ink": "path/to/ink.png", "flattened": true,
  "spine": [[x, y], [x, y]], "straightness": { "flat": 0.4, "corners": 1.9 }, "size": [1024, 768] }
```

`id` is the sample's id (the frame path, the file stem or the manifest's `file`). Only `id` is
required; a photo with no record, or with `corners: null`, is "not found" and scores a Jaccard index
of 0. Without `--found` the report lists the photos and says nothing was scored.
