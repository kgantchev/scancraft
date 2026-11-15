//! The reference photos (specification §10.2), when they are there.
//!
//! The real documents stay out of the repository (`fixtures/reference/` is ignored by git), so on
//! a machine without them each test says why it did nothing and passes. Set
//! `SCANCRAFT_REFERENCE_DIR` to point somewhere else.

use scancraft_core::image::{BitPlane, Gray8, Plane};
use scancraft_core::recipe::{Quality, TextLine, Word};
use scancraft_io::encode::{EncodedImage, PageImage, encode};
use scancraft_io::import::load;
use scancraft_io::pdf_import::PdfDocument;
use scancraft_io::pdf_write::{DocInfo, PdfPage, write_pdf};
use scancraft_io::text_layer::text_layer;
use std::path::{Path, PathBuf};

fn reference_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("SCANCRAFT_REFERENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/reference"));
    dir.is_dir().then_some(dir)
}

fn files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.contains(&e.to_ascii_lowercase().as_str())))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn the_reference_photos_import_encode_and_export() {
    let Some(dir) = reference_dir() else {
        eprintln!("skipped: no reference set here (fixtures/reference/ or SCANCRAFT_REFERENCE_DIR)");
        return;
    };
    let photos = files(&dir, &["jpg", "jpeg", "png", "heic", "heif", "tif", "tiff", "webp"]);
    if photos.is_empty() {
        eprintln!("skipped: the reference set holds no photos");
        return;
    }
    let mut pages = Vec::new();
    for path in &photos {
        let bytes = std::fs::read(path).unwrap();
        let loaded = load(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!loaded.is_empty(), "{}", path.display());
        for image in &loaded {
            let rgb = &image.rgb;
            assert!(rgb.width() >= 100 && rgb.height() >= 100, "{}", path.display());
            // A photo of a page is not blank.
            let first = rgb.data()[0];
            assert!(rgb.data().iter().any(|p| p != &first), "{} is one flat colour", path.display());
            let grey: Gray8 = Plane::from_vec(
                rgb.width(),
                rgb.height(),
                rgb.data().iter().map(|p| ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8).collect(),
            )
            .unwrap();
            let mut bits = BitPlane::new(rgb.width(), rgb.height()).unwrap();
            for y in 0..rgb.height() {
                for x in 0..rgb.width() {
                    bits.set(x, y, grey.get(x, y).unwrap_or(255) < 110);
                }
            }
            let (w, h) = (f64::from(rgb.width()) * 72.0 / 200.0, f64::from(rgb.height()) * 72.0 / 200.0);
            let words = vec![TextLine {
                words: vec![Word { text: "Reference".to_string(), rect: [50.0, 50.0, 300.0, 110.0], confidence: 0.9, source: "test".to_string() }],
            }];
            let layer = text_layer(&words, (rgb.width(), rgb.height()), (w, h)).unwrap();
            for (look, quality) in
                [(PageImage::Colour(rgb.clone()), Quality::Balanced), (PageImage::Grey(grey), Quality::Small), (PageImage::Bits(bits), Quality::Best)]
            {
                let encoded = encode(&look, quality).unwrap();
                assert!(matches!(&encoded, EncodedImage::Jpeg { .. } | EncodedImage::Jbig2 { .. }));
                pages.push(PdfPage { image: encoded, size_pt: (w, h), image_rect_pt: [0.0, 0.0, w, h], text: Some(layer.clone()) });
            }
        }
    }
    let out = std::env::temp_dir().join(format!("scancraft-reference-{}.pdf", std::process::id()));
    write_pdf(&pages, &DocInfo { title: "reference".to_string(), producer: "ScanCraft test (algorithms 1)".to_string() }, &out).unwrap();
    let written = std::fs::read(&out).unwrap();
    let doc = PdfDocument::open(written).unwrap();
    assert_eq!(doc.page_count() as usize, pages.len());
    assert!(doc.render(0).is_ok());
    let _ = std::fs::remove_file(out);
    eprintln!("checked {} photos, {} pages", photos.len(), pages.len());
}

#[test]
fn the_reference_pdfs_open_and_render() {
    let Some(dir) = reference_dir() else {
        eprintln!("skipped: no reference set here (fixtures/reference/ or SCANCRAFT_REFERENCE_DIR)");
        return;
    };
    let pdfs = files(&dir, &["pdf"]);
    if pdfs.is_empty() {
        eprintln!("skipped: the reference set holds no PDFs");
        return;
    }
    for path in &pdfs {
        let doc = PdfDocument::open(std::fs::read(path).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(doc.page_count() >= 1, "{}", path.display());
        for page in 0..doc.page_count().min(3) {
            let image = doc.render(page).unwrap_or_else(|e| panic!("{} page {page}: {e}", path.display()));
            assert!(u64::from(image.width()) * u64::from(image.height()) <= 40_000_000);
            let first = image.data()[0];
            assert!(image.data().iter().any(|p| p != &first), "{} page {page} is one flat colour", path.display());
        }
    }
}

#[test]
fn an_iphone_heic_decodes_upright() {
    let Some(dir) = reference_dir() else {
        eprintln!("skipped: no reference set here");
        return;
    };
    let heics = files(&dir, &["heic", "heif"]);
    if heics.is_empty() {
        eprintln!("skipped: the reference set holds no HEIC files (A1: a 12-megapixel grid HEIC and a 48-megapixel HEIC)");
        return;
    }
    for path in heics {
        let loaded = load(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let image = &loaded[0].rgb;
        // iPhone photos are taken sideways and carry a rotation; upright, a portrait page is taller.
        eprintln!("{}: {} × {}", path.display(), image.width(), image.height());
        assert!(u64::from(image.width()) * u64::from(image.height()) >= 1_000_000);
    }
}
