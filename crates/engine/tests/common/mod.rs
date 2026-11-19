//! Shared helpers for the engine's integration tests.
#![allow(dead_code)]

use craft_settings::Location;
use scancraft_core::engine_api::{EngineApi, Notice};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{Gray8, Rgb8};
use scancraft_core::recipe::{PageId, PhotoId, RecognizedText, TextLine, Word};
use scancraft_core::traits::{PageDetector, RecognizeOptions, Recognizer, ServiceError, UprightVote, UprightVoter};
use scancraft_engine::{Engine, EngineOptions, Services};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A folder under the system's temporary folder, removed when it goes out of scope.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "scancraft-engine-test-{name}-{}-{}-{nanos}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- pictures ----

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in bytes {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend(data);
    out.extend(&body);
    out.extend(crc32(&body).to_be_bytes());
}

/// A PNG of `w` × `h` pixels, `pixel(x, y)` each, written without compression.
pub fn png(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend(pixel(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        z.extend((block.len() as u16).to_le_bytes());
        z.extend((!(block.len() as u16)).to_le_bytes());
        z.extend(*block);
    }
    z.extend(adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A dark desk with a lighter sheet on it; `seed` changes the desk so each file is different.
pub fn desk_png(seed: u8) -> Vec<u8> {
    png(160, 120, |x, y| {
        let on_sheet = (24..136).contains(&x) && (16..104).contains(&y);
        if on_sheet { [235, 232, 225] } else { [40 + seed, 38, 36 + (x % 7) as u8] }
    })
}

/// The 960 × 720 fixture of the repository, and the outline its truth file gives.
pub fn flat_fixture() -> (Vec<u8>, Quad) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let bytes = std::fs::read(root.join("flat.png")).unwrap();
    let truth: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("flat.json")).unwrap()).unwrap();
    let c = |i: usize| Point::new(truth["corners"][i]["x"].as_f64().unwrap(), truth["corners"][i]["y"].as_f64().unwrap());
    (bytes, Quad([c(0), c(1), c(2), c(3)]))
}

pub fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

// ---- services ----

/// A detector that always finds `quad`.
pub struct FixedDetector(pub Quad);

impl PageDetector for FixedDetector {
    fn detect(&self, _: &Rgb8) -> Result<Option<Quad>, ServiceError> {
        Ok(Some(self.0))
    }
}

/// A detector that waits for a release before it answers.
pub struct HeldDetector {
    pub quad: Quad,
    pub gate: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl PageDetector for HeldDetector {
    fn detect(&self, _: &Rgb8) -> Result<Option<Quad>, ServiceError> {
        if let Ok(gate) = self.gate.lock() {
            let _ = gate.recv_timeout(Duration::from_secs(20));
        }
        Ok(Some(self.quad))
    }
}

/// A detector that takes its time over the photo whose first pixel's red is `marker`, so that
/// another photo's page can join the document before it.
pub struct SlowFor {
    pub quad: Quad,
    pub marker: u8,
    pub delay: Duration,
}

impl PageDetector for SlowFor {
    fn detect(&self, photo: &Rgb8) -> Result<Option<Quad>, ServiceError> {
        if photo.get(0, 0).is_some_and(|p| p[0] == self.marker) {
            std::thread::sleep(self.delay);
        }
        Ok(Some(self.quad))
    }
}

/// A recogniser that holds the worker until it is released, then reads one word: with one worker
/// thread, whatever is queued behind it has to wait.
pub struct GatedWord {
    pub gate: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Recognizer for GatedWord {
    fn name(&self) -> &str {
        "fake"
    }

    fn recognize(&self, page: &Gray8, options: &RecognizeOptions) -> Result<RecognizedText, ServiceError> {
        if let Ok(gate) = self.gate.lock() {
            let _ = gate.recv_timeout(Duration::from_secs(20));
        }
        OneWord.recognize(page, options)
    }
}

/// An upright voter that holds its job until it is released, then votes, surely, for a half turn.
/// It lets a test finish the page's other background work first.
pub struct GatedHalfTurn {
    pub gate: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl UprightVoter for GatedHalfTurn {
    fn vote(&self, _: &Gray8) -> Result<UprightVote, ServiceError> {
        if let Ok(gate) = self.gate.lock() {
            // Long, so that a slow machine finishes the other work first: the wait ends when the
            // test releases the gate, or drops it, which it does however it ends.
            let _ = gate.recv_timeout(Duration::from_secs(300));
        }
        Ok(UprightVote { score: [0.0, 0.0, 10.0, 0.0], chars: [0, 0, 100, 0] })
    }
}

/// A recogniser that reads one word.
pub struct OneWord;

impl Recognizer for OneWord {
    fn name(&self) -> &str {
        "fake"
    }

    fn recognize(&self, page: &Gray8, _: &RecognizeOptions) -> Result<RecognizedText, ServiceError> {
        let (w, h) = (page.width() as f32, page.height() as f32);
        Ok(RecognizedText {
            engine: "fake".to_string(),
            engine_version: "1".to_string(),
            lines: vec![TextLine {
                words: vec![Word {
                    text: "hello".to_string(),
                    rect: [w * 0.1, h * 0.1, w * 0.4, h * 0.15],
                    confidence: 0.9,
                    source: "fake".to_string(),
                }],
            }],
        })
    }
}

pub fn services(quad: Quad) -> Services {
    Services { detector: Some(Arc::new(FixedDetector(quad))), recognizer: Some(Arc::new(OneWord)), upright: None }
}

/// The outline `desk_png` has its sheet at, for a detector.
pub fn desk_quad() -> Quad {
    Quad([Point::new(24.0, 16.0), Point::new(136.0, 16.0), Point::new(136.0, 104.0), Point::new(24.0, 104.0)])
}

// ---- engines ----

pub fn open_with(root: &Path, name: &str, services: Services, threads: usize) -> Engine {
    let mut options = EngineOptions::new(root.join(name), services);
    options.settings = Location::Memory;
    options.threads = threads;
    options.sessions_root = Some(root.to_path_buf());
    Engine::open(options).unwrap()
}

pub fn open(root: &TempDir, services: Services) -> Engine {
    open_with(root.path(), "Scan A", services, 2)
}

/// Polls until the engine has nothing left to do, collecting every notice it showed on the way.
pub fn settle(engine: &mut Engine) -> Vec<Notice> {
    let mut all = Vec::new();
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        engine.poll();
        all.extend(engine.view().notices);
        if !engine.view().busy {
            engine.poll();
            all.extend(engine.view().notices);
            break;
        }
        assert!(Instant::now() < end, "the engine did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
    all
}

pub fn texts(notices: &[Notice]) -> Vec<String> {
    notices.iter().map(|n| n.text.clone()).collect()
}

/// The page joins the document by itself and the finished page opens. Many tests are about Crop,
/// so they go on to it: the first photo's page, once, as the first photo used to open.
pub fn open_crop_on_first_page(engine: &mut Engine) {
    if engine.view().crop.is_none()
        && let Some(item) = engine.view().tray.first()
    {
        engine.dispatch(scancraft_core::command::Command::PageOpenCrop { page: item.page }).unwrap();
    }
}

/// The photo on Screen 2 and its first page.
pub fn current(engine: &Engine) -> (PhotoId, PageId) {
    let crop = engine.view().crop.expect("Screen 2 should be showing");
    (crop.photo, crop.outlines[0].page)
}
