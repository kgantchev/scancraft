//! A21: the invisible text layer, ported from PdfCraft's `text_layer`, `TextPlan` and Type3
//! registrar (specification A21, §5.3).

use crate::IoError;
use scancraft_core::recipe::TextLine;
use scancraft_core::{Step, not_implemented};

/// A page's text layer: the content stream `/OCR BMC q BT 3 Tr … ET Q EMC` and the fonts it uses.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayer {
    pub content: Vec<u8>,
    /// Fonts by resource name; their objects are written by A23.
    pub fonts: Vec<LayerFont>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayerFont {
    /// Standard Helvetica with WinAnsi encoding.
    Helvetica { name: String },
    /// A Type3 font with empty glyphs and a ToUnicode map.
    Type3 { name: String, to_unicode: Vec<(u8, char)> },
}

/// Builds the layer for words in page pixels on a page of `page_px` pixels and `page_pt` points.
pub fn text_layer(lines: &[TextLine], page_px: (u32, u32), page_pt: (f64, f64)) -> Result<TextLayer, IoError> {
    let _ = (lines, page_px, page_pt);
    not_implemented(Step::A21)
}
