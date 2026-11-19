//! The `detect`, `flatten`, `page` and `pdf` subcommands (specification §6.9): the whole pipeline
//! without a window. They drive the real engine by commands, in a session folder made for the run
//! and removed after it, so the tool and the app cannot disagree about what a photo comes to.
//!
//! A usage error exits with 64, a failed job with 1; nothing panics on junk.

use crate::batch::{Flags, parse_flags, usage};
use craft_settings::Location;
use scancraft_core::command::Command;
use scancraft_core::engine_api::EngineApi;
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::{ExportOptions, FlattenMode, Look, Page, PageOrder, Quality};
use scancraft_engine::cache::Cache;
use scancraft_engine::jobs::CancellationToken;
use scancraft_engine::pipeline::{Ctx, RenderSpec, final_size, load_photo, output_dpi, page_geometry, render_page};
use scancraft_engine::{Engine, EngineOptions, Services};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const DETECT_USAGE: &str = "usage: scancraft-cli detect <photo>... [--book] [--order ltr|rtl] [--json]";
const FLATTEN_USAGE: &str = "usage: scancraft-cli flatten <photo>... [--flatten auto|on|off] [--json]";
const PAGE_USAGE: &str =
    "usage: scancraft-cli page <photo> [--look color|gray|bw|original] [--strength 0-100] [--flatten auto|on|off] [--threads N] --out FILE.png";
const PDF_USAGE: &str = "usage: scancraft-cli pdf <photo>... [--look ..] [--strength N] [--flatten ..] [--threads N] [--book] [--order ltr|rtl] [--quality small|balanced|best] [--ocr off|auto] --out FILE.pdf";

/// The longest one run waits for the engine's jobs.
const SETTLE_LIMIT: Duration = Duration::from_secs(1800);

/// An engine with a session folder of its own.
struct Run {
    engine: Engine,
    dir: PathBuf,
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn open_run(services: Services, order: PageOrder, book: bool, threads: usize) -> Result<Run, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("scancraft-cli-{}-{nanos}", std::process::id()));
    let mut options = EngineOptions::new(dir.join("session"), services);
    options.settings = Location::Memory;
    options.sessions_root = Some(dir.clone());
    options.threads = threads;
    let mut engine = Engine::open(options).map_err(|e| e.to_string())?;
    let run_dir = dir.clone();
    engine.set_settings(|s| s.book_page_order = order);
    let run = Run { engine, dir: run_dir };
    let mut run = run;
    if book {
        run.engine.dispatch(Command::CropBook { on: true })?;
    }
    Ok(run)
}

/// Polls until the engine has nothing left to do; its messages come back.
fn settle(engine: &mut Engine) -> Result<Vec<String>, String> {
    let end = Instant::now() + SETTLE_LIMIT;
    let mut notes: Vec<String> = Vec::new();
    loop {
        engine.poll();
        for n in engine.view().notices {
            if !notes.contains(&n.text) {
                notes.push(n.text);
            }
        }
        if !engine.view().busy {
            return Ok(notes);
        }
        if Instant::now() > end {
            return Err("the engine did not finish in time".to_string());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn services() -> Services {
    // Vision is the app's; the tool uses the classical detector alone. Text recognition needs
    // ocrs's model files, which the tool doesn't ship, so `--ocr auto` says so (see `pdf`).
    Services::default()
}

fn parse_order(flags: &Flags) -> Result<PageOrder, String> {
    match flags.value("--order") {
        None | Some("ltr") => Ok(PageOrder::LeftToRight),
        Some("rtl") => Ok(PageOrder::RightToLeft),
        Some(other) => Err(format!("--order is ltr or rtl, not {other:?}")),
    }
}

fn parse_flatten(flags: &Flags) -> Result<FlattenMode, String> {
    match flags.value("--flatten") {
        None | Some("auto") => Ok(FlattenMode::Auto),
        Some("on") => Ok(FlattenMode::On),
        Some("off") => Ok(FlattenMode::Off),
        Some(other) => Err(format!("--flatten is auto, on or off, not {other:?}")),
    }
}

fn parse_look(flags: &Flags) -> Result<Look, String> {
    match flags.value("--look") {
        None | Some("color") => Ok(Look::ColorDocument),
        Some("gray") | Some("grey") => Ok(Look::Grayscale),
        Some("bw") => Ok(Look::BlackWhite),
        Some("original") => Ok(Look::Original),
        Some(other) => Err(format!("--look is color, gray, bw or original, not {other:?}")),
    }
}

fn parse_threads(flags: &Flags) -> Result<usize, String> {
    flags
        .value("--threads")
        .map_or(Ok(0), |v| v.parse::<usize>().ok().filter(|n| *n <= 64).ok_or_else(|| "--threads is a whole number from 0 to 64".to_string()))
}

fn parse_strength(flags: &Flags) -> Result<Option<u8>, String> {
    flags
        .value("--strength")
        .map(|v| v.parse::<u8>().ok().filter(|n| *n <= 100).ok_or_else(|| "--strength is a whole number from 0 to 100".to_string()))
        .transpose()
}

/// The positional arguments (photo paths) and the flags that follow them.
fn split_args(args: &[String], with_value: &[&str], switches: &[&str]) -> Result<(Vec<String>, Flags), String> {
    let mut paths = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg.starts_with("--") {
            rest.push(arg.clone());
            if with_value.contains(&arg.as_str())
                && let Some(v) = it.next()
            {
                rest.push(v.clone());
            }
        } else {
            paths.push(arg.clone());
        }
    }
    if paths.is_empty() {
        return Err("a photo is needed".to_string());
    }
    Ok((paths, parse_flags(&rest, with_value, switches)?))
}

fn corners_json(q: &scancraft_core::geom::Quad) -> Value {
    json!(q.0.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>())
}

fn stem(path: &str) -> String {
    Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string()
}

/// One photo through detection (and, when `flatten`, the crop and A29): the record `eval --found`
/// reads, with the extra fields the tool reports.
fn analyse(path: &str, order: PageOrder, book: bool, mode: FlattenMode, flatten: bool) -> Result<Value, String> {
    let mut run = open_run(services(), order, book, 0)?;
    let start = Instant::now();
    run.engine.dispatch(Command::ImportAdd { paths: vec![path.to_string()] })?;
    let mut notes = settle(&mut run.engine)?;
    let detect_ms = start.elapsed().as_secs_f64() * 1000.0;
    // The page joined the document by itself; Crop shows the outline detection found.
    if let Some(item) = run.engine.view().tray.first() {
        run.engine.dispatch(Command::PageOpenCrop { page: item.page })?;
    }
    let view = run.engine.view();
    let Some(crop) = view.crop else {
        return Err(format!("{path}: {}", notes.pop().unwrap_or_else(|| "the file could not be imported".to_string())));
    };
    let outlines: Vec<Value> = crop
        .outlines
        .iter()
        .map(|o| {
            json!({
                "corners": corners_json(&o.corners),
                "origin": serde_json::to_value(o.corners_from).unwrap_or(Value::Null),
                "sides": serde_json::to_value(&o.sides).unwrap_or(Value::Null),
                "half": serde_json::to_value(o.half).unwrap_or(Value::Null),
            })
        })
        .collect();
    let found = crop.outlines.first().is_some_and(|o| !matches!(o.corners_from, scancraft_core::recipe::CornerOrigin::NotFound));
    let mut record = json!({
        "id": stem(path),
        "file": path,
        "size": [crop.photo_size.0, crop.photo_size.1],
        "found": found,
        "corners": if found { crop.outlines.first().map_or(Value::Null, |o| corners_json(&o.corners)) } else { Value::Null },
        "outlines": outlines,
        "spine": crop.spine.map(|s| json!([[s[0].x, s[0].y], [s[1].x, s[1].y]])),
        "detect_ms": detect_ms,
    });
    if flatten {
        for o in &crop.outlines {
            run.engine.dispatch(Command::CropSetFlatten { page: o.page, mode })?;
        }
        let t = Instant::now();
        run.engine.dispatch(Command::CropApply { photo: crop.photo })?;
        notes.extend(settle(&mut run.engine)?);
        let flatten_ms = t.elapsed().as_secs_f64() * 1000.0;
        let session = run.engine.session();
        let pages: Vec<&Page> = session.file().pages.iter().collect();
        let decisions: Vec<Value> = pages
            .iter()
            .map(|p| match &p.sheet {
                Some(fit) => json!({ "flattened": fit.decision.flattened, "decision": serde_json::to_value(&fit.decision).unwrap_or(Value::Null) }),
                None => json!({ "flattened": false, "decision": Value::Null }),
            })
            .collect();
        record["flattened"] = json!(pages.first().and_then(|p| p.sheet.as_ref()).is_some_and(|f| f.decision.flattened));
        record["pages"] = Value::Array(decisions);
        record["flatten_ms"] = json!(flatten_ms);
    }
    record["notes"] = json!(notes);
    Ok(record)
}

/// `detect <photo>... [--book] [--order ..] [--json]` and `flatten <photo>... [--flatten ..] [--json]`.
pub fn detect_or_flatten(args: &[String], flatten: bool) -> ExitCode {
    let help = if flatten { FLATTEN_USAGE } else { DETECT_USAGE };
    let parsed = split_args(args, &["--order", "--flatten"], &["--json", "--book"]).and_then(|(paths, flags)| {
        let mode = if flatten { parse_flatten(&flags)? } else { FlattenMode::Auto };
        Ok((paths, parse_order(&flags)?, flags.switches.iter().any(|s| s == "--book"), mode, flags.switches.iter().any(|s| s == "--json")))
    });
    let (paths, order, book, mode, as_json) = match parsed {
        Ok(v) => v,
        Err(m) => return usage(&m, help),
    };
    let mut records = Vec::new();
    let mut failed = false;
    for path in &paths {
        match analyse(path, order, book, mode, flatten) {
            Ok(r) => records.push(r),
            Err(m) => {
                eprintln!("scancraft-cli: {m}");
                failed = true;
            }
        }
    }
    if as_json {
        println!("{}", serde_json::to_string_pretty(&Value::Array(records)).unwrap_or_default());
    } else {
        for r in &records {
            let corners = r["corners"].to_string();
            println!("{}: {}", r["file"].as_str().unwrap_or(""), if r["found"] == true { corners } else { "page not found".to_string() });
            if flatten {
                println!("  flattened: {}", r["flattened"]);
            }
        }
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// Opens a run, imports `paths` and takes every photo through Screens 2 and 3 with the same recipe,
/// so every page is in the tray.
fn build_session(
    paths: &[String],
    order: PageOrder,
    book: bool,
    mode: FlattenMode,
    look: Look,
    strength: Option<u8>,
    threads: usize,
) -> Result<Run, String> {
    let mut run = open_run(services(), order, book, threads)?;
    run.engine.dispatch(Command::ImportAdd { paths: paths.to_vec() })?;
    settle(&mut run.engine)?;
    // Every photo's page joined the document by itself, in the order of the photos whichever was
    // read first, so the pages come out in the order of the command line however many threads
    // there are. Each photo is cropped again with the recipe's flatten mode.
    let photos: Vec<_> = run.engine.session().file().photos.iter().map(|p| p.id).collect();
    for photo in photos {
        let Some(page) = run.engine.session().pages_of(photo).first().map(|p| p.id) else { continue };
        run.engine.dispatch(Command::PageOpenCrop { page })?;
        if let Some(crop) = run.engine.view().crop {
            for o in &crop.outlines {
                run.engine.dispatch(Command::CropSetFlatten { page: o.page, mode })?;
            }
            run.engine.dispatch(Command::CropApply { photo })?;
            settle(&mut run.engine)?;
        }
    }
    if !run.engine.session().file().pages.is_empty() {
        let value = strength.unwrap_or_else(|| look.default_strength().unwrap_or(0));
        run.engine.dispatch(Command::PageSetLookAll { look, strength: value })?;
        settle(&mut run.engine)?;
    }
    if run.engine.session().file().pages.is_empty() {
        return Err("no page came out of the photos".to_string());
    }
    Ok(run)
}

fn quality(flags: &Flags) -> Result<Quality, String> {
    match flags.value("--quality") {
        None | Some("balanced") => Ok(Quality::Balanced),
        Some("small") => Ok(Quality::Small),
        Some("best") => Ok(Quality::Best),
        Some(other) => Err(format!("--quality is small, balanced or best, not {other:?}")),
    }
}

/// `pdf <photo>... --out FILE.pdf`
pub fn pdf(args: &[String]) -> ExitCode {
    let flag_values = ["--look", "--strength", "--flatten", "--order", "--quality", "--ocr", "--out", "--threads"];
    let parsed = split_args(args, &flag_values, &["--book"]).and_then(|(paths, flags)| {
        let ocr = match flags.value("--ocr") {
            None | Some("off") => false,
            Some("auto") => true,
            Some(other) => return Err(format!("--ocr is off or auto, not {other:?}")),
        };
        let out = flags.required("--out")?.to_string();
        Ok((
            paths,
            parse_order(&flags)?,
            flags.switches.iter().any(|s| s == "--book"),
            parse_flatten(&flags)?,
            parse_look(&flags)?,
            parse_strength(&flags)?,
            quality(&flags)?,
            ocr,
            out,
            parse_threads(&flags)?,
        ))
    });
    let (paths, order, book, mode, look, strength, quality, ocr, out, threads) = match parsed {
        Ok(v) => v,
        Err(m) => return usage(&m, PDF_USAGE),
    };
    let result = (|| -> Result<Vec<String>, String> {
        let mut run = build_session(&paths, order, book, mode, look, strength, threads)?;
        if ocr {
            eprintln!("scancraft-cli: --ocr auto needs text-recognition models, which this build doesn't carry; the PDF has no text layer");
        }
        let options = ExportOptions { quality, searchable: false, ..ExportOptions::default() };
        run.engine.dispatch(Command::ExportPdf { path: out.clone(), options })?;
        settle(&mut run.engine)
    })();
    match result {
        Ok(notes) => {
            for n in notes {
                eprintln!("scancraft-cli: {n}");
            }
            println!("wrote {out}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("scancraft-cli pdf: {m}");
            ExitCode::from(1)
        }
    }
}

/// The first page of the session at its output size.
fn render_first(run: &Run) -> Result<(scancraft_engine::pipeline::PageOutput, Rgb8), String> {
    let session = run.engine.session();
    let page = session.file().pages.first().ok_or("no page came out of the photo")?.clone();
    let item = session.photo(page.photo).ok_or("the page's photo is gone")?.clone();
    let cache = Cache::new(session.cache_dir()).map_err(|e| e.to_string())?;
    let ctx = Ctx {
        cache: std::sync::Arc::new(cache),
        sources: session.sources_dir(),
        services: run.engine.services().clone(),
        settings: run.engine.settings().clone(),
    };
    let token = CancellationToken::new();
    let loaded = load_photo(&ctx, &item.source, &item.origin, &token).map_err(|e| e.to_string())?;
    let photo = loaded.image;
    let geometry = page_geometry(&page, (photo.width(), photo.height()), item.focal, &ctx.settings).map_err(|e| e.to_string())?;
    let dpi = output_dpi(Quality::Balanced, page.look);
    let size = final_size(&geometry, page.quarter_turns, dpi, page.look);
    let (out, _) =
        render_page(&ctx, &photo, &item, &page, RenderSpec { size, dpi: f64::from(dpi), preview: false }, &token).map_err(|e| e.to_string())?;
    let rgb = out.to_rgb8().map_err(|e| e.to_string())?;
    Ok((out, rgb))
}

/// `page <photo> --out FILE.png`
pub fn page(args: &[String]) -> ExitCode {
    let flag_values = ["--look", "--strength", "--flatten", "--out", "--threads"];
    let parsed = split_args(args, &flag_values, &[]).and_then(|(paths, flags)| {
        if paths.len() != 1 {
            return Err("page takes one photo".to_string());
        }
        Ok((
            paths,
            parse_flatten(&flags)?,
            parse_look(&flags)?,
            parse_strength(&flags)?,
            flags.required("--out")?.to_string(),
            parse_threads(&flags)?,
        ))
    });
    let (paths, mode, look, strength, out, threads) = match parsed {
        Ok(v) => v,
        Err(m) => return usage(&m, PAGE_USAGE),
    };
    let result = (|| -> Result<(u32, u32), String> {
        let run = build_session(&paths, PageOrder::LeftToRight, false, mode, look, strength, threads)?;
        let (_, rgb) = render_first(&run)?;
        let bytes: Vec<u8> = rgb.data().iter().flat_map(|p| *p).collect();
        image::save_buffer(&out, &bytes, rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
        Ok((rgb.width(), rgb.height()))
    })();
    match result {
        Ok((w, h)) => {
            println!("wrote {out} ({w} x {h})");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("scancraft-cli page: {m}");
            ExitCode::from(1)
        }
    }
}
