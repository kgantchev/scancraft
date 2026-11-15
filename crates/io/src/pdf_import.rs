//! A24: PDF import through craft-pdf's bounded reader and rasteriser.
//!
//! A page is rendered at 300 DPI, honouring its `/Rotate`, at a lower DPI when 300 would pass 40
//! megapixels (or the rasteriser's own side limit), and composited over white. The cap of 500
//! pages per file belongs to the caller, which can say how many it left out: `page_count` gives
//! the real number.

use crate::IoError;
use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::raster::{Options, RenderCache};
use craft_pdf::syntax::{DecryptionError, LoadPdfError, Pdf};
use craft_pdf::{OpenError, open};
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSlice, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, MAX_PAGE_PIXELS, Plane, Rgb8};
use std::sync::Arc;

/// The pages an import takes from one PDF (A24).
pub const MAX_PAGES: usize = 500;
/// The resolution pages are rendered at, in dots per inch.
const DPI: f64 = 300.0;
/// A PDF's user unit.
const POINTS_PER_INCH: f64 = 72.0;

/// An opened PDF.
pub struct PdfDocument {
    pdf: Pdf,
}

impl PdfDocument {
    pub fn open(bytes: Vec<u8>) -> Result<PdfDocument, IoError> {
        match open(Arc::new(bytes), None) {
            Ok(pdf) => Ok(PdfDocument { pdf }),
            // A password, or an encryption we can't read, both leave the file unusable.
            Err(OpenError::Load(LoadPdfError::Decryption(
                DecryptionError::PasswordProtected
                | DecryptionError::InvalidEncryption
                | DecryptionError::UnsupportedAlgorithm
                | DecryptionError::MissingIDEntry,
            ))) => Err(IoError::Encrypted),
            Err(OpenError::InputLimit { .. }) => Err(IoError::Unsupported("the PDF is too large to open".to_string())),
            Err(OpenError::Load(LoadPdfError::Invalid)) => Err(IoError::Unsupported("the PDF could not be read".to_string())),
            Err(OpenError::Allocation | OpenError::BackendPanic) => Err(IoError::Fs("the PDF could not be opened".to_string())),
        }
    }

    /// The pages in the file; a caller that imports at most [`MAX_PAGES`] subtracts.
    pub fn page_count(&self) -> u32 {
        u32::try_from(self.pdf.pages().len()).unwrap_or(u32::MAX)
    }

    /// Renders page `index` at 300 DPI, lowered to fit 40 megapixels, composited over white.
    pub fn render(&self, index: u32) -> Result<Rgb8, IoError> {
        let page = self.pdf.pages().get(index as usize).ok_or_else(|| IoError::Unsupported("the PDF has no such page".to_string()))?;
        let (w_pt, h_pt) = page.render_dimensions();
        let usable = |v: f32| v.is_finite() && v > 0.0;
        if !usable(w_pt) || !usable(h_pt) {
            return Err(IoError::Unsupported("the PDF page has no usable size".to_string()));
        }
        let settings = InterpreterSettings::default();
        let mut scale = DPI / POINTS_PER_INCH;
        let side_limit = f64::from(craft_pdf::raster::MAX_SIDE);
        // Lower the DPI until the page fits both the 40 megapixels and the rasteriser's own side
        // limit. The first step nearly always lands; the loop only absorbs rounding to pixels.
        for _ in 0..6 {
            let (w, h) = ((f64::from(w_pt) * scale).ceil().max(1.0), (f64::from(h_pt) * scale).ceil().max(1.0));
            let pixels = w * h;
            if pixels > MAX_PAGE_PIXELS as f64 || w.max(h) > side_limit {
                // A little under, so rounding up to whole pixels doesn't land just over again.
                scale *= (MAX_PAGE_PIXELS as f64 / pixels).sqrt().min(side_limit / w.max(h)) * 0.995;
                continue;
            }
            let options = Options { scale: [scale as f32, scale as f32], ..Options::default() };
            let raster =
                craft_pdf::raster::render(page, &RenderCache::new(), &settings, &options).map_err(|e| IoError::Unsupported(e.to_string()))?;
            return over_white(raster.bytes(), raster.width, raster.height);
        }
        Err(IoError::Unsupported("the PDF page is too large to render".to_string()))
    }
}

/// Composites premultiplied RGBA8 over white: c′ = c + (1 − a)·255, since c is already a·c.
fn over_white(rgba: &[u8], width: u32, height: u32) -> Result<Rgb8, IoError> {
    let (w, h) = (width as usize, height as usize);
    if rgba.len() != w * h * 4 {
        return Err(IoError::Unsupported("the rendered page has an unexpected size".to_string()));
    }
    let mut out = vec![[0u8; 3]; w * h];
    let band = w * BAND_ROWS as usize;
    out.par_chunks_mut(band.max(1)).zip(rgba.par_chunks((band * 4).max(4))).for_each(|(dst, src)| {
        for (pixel, px) in dst.iter_mut().zip(src.as_chunks::<4>().0.iter()) {
            let inverse = 255 - u16::from(px[3]);
            *pixel = [0, 1, 2].map(|c| (u16::from(px[c]) + inverse).min(255) as u8);
        }
    });
    Ok(Plane::from_vec(width, height, out)?)
}

#[cfg(test)]
mod tests {
    use super::{PdfDocument, over_white};
    use crate::IoError;

    /// A PDF of `pages`, each (width, height, rotate, content stream), made by hand with a
    /// cross-reference table.
    fn pdf(pages: &[(u32, u32, u32, &str)]) -> Vec<u8> {
        pdf_with_trailer(pages, "")
    }

    /// The same with extra entries in the trailer dictionary.
    fn pdf_with_trailer(pages: &[(u32, u32, u32, &str)], trailer: &str) -> Vec<u8> {
        let mut objects: Vec<String> = Vec::new();
        // 1 catalog, 2 pages, then three objects a page: page, content.
        let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
        objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
        objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()));
        for (i, (w, h, rotate, content)) in pages.iter().enumerate() {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Rotate {rotate} /Contents {} 0 R /Resources << >> >>",
                4 + 2 * i
            ));
            objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    /// A red rectangle across the lower-left quarter of the page, in user space.
    fn red_corner(w: u32, h: u32) -> String {
        format!("1 0 0 rg 0 0 {} {} re f", w / 2, h / 2)
    }

    #[test]
    fn a_three_page_pdf_gives_three_images_in_order() {
        let doc =
            PdfDocument::open(pdf(&[(200, 100, 0, "0 g 0 0 50 50 re f"), (100, 200, 0, "0 0 1 rg 0 0 100 100 re f"), (150, 150, 0, "")])).unwrap();
        assert_eq!(doc.page_count(), 3);
        let sizes: Vec<(u32, u32)> = (0..3).map(|i| doc.render(i).map(|p| (p.width(), p.height())).unwrap()).collect();
        // 300 DPI: 200 × 100 pt is 833 × 417 px (the rasteriser rounds down).
        assert_eq!(sizes, vec![(833, 416), (416, 833), (625, 625)]);
        // The first page is black in its lower-left corner, the second blue in its lower half.
        let first = doc.render(0).unwrap();
        assert_eq!(first.get(10, 400), Some([0, 0, 0]));
        assert_eq!(first.get(800, 10), Some([255, 255, 255]));
        let second = doc.render(1).unwrap();
        assert_eq!(second.get(200, 800), Some([0, 0, 255]));
        assert_eq!(second.get(200, 100), Some([255, 255, 255]));
        assert!(doc.render(3).is_err());
    }

    #[test]
    fn a_rotated_page_arrives_upright() {
        // A landscape page of 200 × 100 pt rotated by 90 is portrait, 100 × 200 pt on the screen,
        // and the red corner that was bottom-left turns to the top-left.
        let upright = PdfDocument::open(pdf(&[(200, 100, 0, &red_corner(200, 100))])).unwrap().render(0).unwrap();
        let rotated = PdfDocument::open(pdf(&[(200, 100, 90, &red_corner(200, 100))])).unwrap().render(0).unwrap();
        assert_eq!((upright.width(), upright.height()), (833, 416));
        assert_eq!((rotated.width(), rotated.height()), (416, 833));
        assert_eq!(upright.get(10, 400), Some([255, 0, 0]));
        // After a clockwise quarter turn the bottom-left corner is at the top-left.
        assert_eq!(rotated.get(10, 10), Some([255, 0, 0]));
        assert_eq!(rotated.get(10, 820), Some([255, 255, 255]));
        let flipped = PdfDocument::open(pdf(&[(200, 100, 180, &red_corner(200, 100))])).unwrap().render(0).unwrap();
        assert_eq!(flipped.get(820, 10), Some([255, 0, 0]));
    }

    #[test]
    fn the_page_is_rendered_at_300_dpi_and_lowered_to_fit() {
        // A4: 595 × 842 pt is 2479 × 3508 px at 300 DPI.
        let a4 = PdfDocument::open(pdf(&[(595, 842, 0, "")])).unwrap().render(0).unwrap();
        assert!((i64::from(a4.width()) - 2479).abs() <= 1 && (i64::from(a4.height()) - 3508).abs() <= 1, "{}x{}", a4.width(), a4.height());
        // A 2 m square page would be 23 600 px a side: lowered to fit 40 megapixels.
        let big = PdfDocument::open(pdf(&[(5669, 5669, 0, "0 g 0 0 100 100 re f")])).unwrap().render(0).unwrap();
        assert!(u64::from(big.width()) * u64::from(big.height()) <= 40_000_000);
        assert!(big.width() >= 5000, "{}", big.width());
        // A long narrow page is held by the rasteriser's side limit, not the pixel count.
        let tall = PdfDocument::open(pdf(&[(100, 14_000, 0, "")])).unwrap().render(0).unwrap();
        assert!(tall.height() <= 8192 && tall.height() >= 8000, "{}", tall.height());
    }

    #[test]
    fn the_render_is_composited_over_white() {
        // A page with no content has a transparent background: it must come out white, not black.
        let blank = PdfDocument::open(pdf(&[(100, 100, 0, "")])).unwrap().render(0).unwrap();
        assert!(blank.data().iter().all(|p| *p == [255, 255, 255]));
        // Premultiplied: half-transparent black over white is mid-grey, and over_white agrees.
        let rgba = [0u8, 0, 0, 128, 64, 0, 0, 128, 0, 0, 0, 0, 10, 20, 30, 255];
        let out = over_white(&rgba, 4, 1).unwrap();
        assert_eq!(out.data(), &[[127, 127, 127], [191, 127, 127], [255, 255, 255], [10, 20, 30]]);
        assert!(over_white(&rgba, 3, 1).is_err());
    }

    #[test]
    fn a_truncated_pdf_fails_cleanly() {
        let bytes = pdf(&[(100, 100, 0, "0 g 0 0 10 10 re f"), (100, 100, 0, "")]);
        for cut in [0, 5, 20, bytes.len() / 3, bytes.len() / 2, bytes.len() - 30] {
            // Either it doesn't open or it opens as best it can; nothing panics.
            match PdfDocument::open(bytes[..cut].to_vec()) {
                Ok(doc) => {
                    for i in 0..doc.page_count() {
                        let _ = doc.render(i);
                    }
                }
                Err(e) => assert!(matches!(e, IoError::Unsupported(_) | IoError::Fs(_) | IoError::Encrypted)),
            }
        }
        assert!(PdfDocument::open(b"not a pdf".to_vec()).is_err());
        assert!(PdfDocument::open(Vec::new()).is_err());
    }

    #[test]
    fn an_encrypted_pdf_is_reported_as_password_protected() {
        // The standard security handler with an owner and a user entry no empty password opens.
        let hex = "0123456789ABCDEF".repeat(4);
        let trailer = format!(
            "/Encrypt << /Filter /Standard /V 1 /R 2 /O <{hex}> /U <{hex}> /P -4 >> /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>]"
        );
        let result = PdfDocument::open(pdf_with_trailer(&[(100, 100, 0, "")], &trailer));
        assert!(matches!(result, Err(IoError::Encrypted)), "{:?}", result.err());
        assert_eq!(IoError::Encrypted.to_string(), "this PDF is password-protected");
    }

    #[test]
    fn a_ten_thousand_page_pdf_stops_cleanly() {
        let pages: Vec<(u32, u32, u32, &str)> = (0..10_000).map(|_| (10, 10, 0, "")).collect();
        let started = std::time::Instant::now();
        let doc = PdfDocument::open(pdf(&pages)).unwrap();
        assert_eq!(doc.page_count(), 10_000);
        // The caller imports at most 500 and can say how many it left out.
        assert_eq!(doc.page_count() as usize - super::MAX_PAGES, 9_500);
        assert!(doc.render(9_999).is_ok());
        assert!(started.elapsed().as_secs() < 60);
    }

    #[test]
    fn one_and_eight_threads_agree() {
        let doc = PdfDocument::open(pdf(&[(300, 200, 0, "1 0 0 rg 10 10 150 100 re f 0 0 1 rg 100 50 150 140 re f")])).unwrap();
        let run = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| doc.render(0).unwrap());
        assert_eq!(run(1), run(8));
    }
}
