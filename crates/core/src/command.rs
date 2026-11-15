//! Every action the user can take, as a command with an id and parameters (specification §6.3).
//!
//! Buttons, menus, shortcuts, the command-line tool and tests all run commands by id. A command
//! run by a program never opens a dialog.

use crate::geom::{Point, Side};
use crate::recipe::{ExportOptions, FlattenMode, Look, PageId, PhotoId};
use serde::{Deserialize, Serialize};

/// Corner index in TL, TR, BR, BL order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub const fn index(self) -> usize {
        match self {
            Corner::TopLeft => 0,
            Corner::TopRight => 1,
            Corner::BottomRight => 2,
            Corner::BottomLeft => 3,
        }
    }
}

/// The commands of §6.3, plus undo and redo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "id", content = "params")]
pub enum Command {
    #[serde(rename = "import.add")]
    ImportAdd { paths: Vec<String> },
    #[serde(rename = "import.discard")]
    ImportDiscard { item: PhotoId },
    #[serde(rename = "crop.set_corner")]
    CropSetCorner { page: PageId, corner: Corner, x: f64, y: f64 },
    #[serde(rename = "crop.move_side")]
    CropMoveSide { page: PageId, side: Side, offset: f64 },
    #[serde(rename = "crop.bend_side")]
    CropBendSide { page: PageId, side: Side, handles: [Point; 3] },
    #[serde(rename = "crop.straighten_side")]
    CropStraightenSide { page: PageId, side: Side },
    #[serde(rename = "crop.set_flatten")]
    CropSetFlatten { page: PageId, mode: FlattenMode },
    #[serde(rename = "crop.book")]
    CropBook { on: bool },
    #[serde(rename = "crop.split")]
    CropSplit { photo: PhotoId, on: bool },
    #[serde(rename = "crop.set_spine")]
    CropSetSpine { photo: PhotoId, top: Point, bottom: Point },
    #[serde(rename = "crop.auto")]
    CropAuto { page: PageId },
    #[serde(rename = "crop.full_image")]
    CropFullImage { page: PageId },
    #[serde(rename = "crop.apply")]
    CropApply { photo: PhotoId },
    #[serde(rename = "page.set_look")]
    PageSetLook { page: PageId, look: Look },
    #[serde(rename = "page.set_strength")]
    PageSetStrength { page: PageId, value: u8 },
    #[serde(rename = "page.rotate")]
    PageRotate { page: PageId, quarter_turns: i8 },
    #[serde(rename = "page.add")]
    PageAdd { page: PageId },
    #[serde(rename = "page.delete")]
    PageDelete { page: PageId },
    #[serde(rename = "page.move")]
    PageMove { from: u32, to: u32 },
    #[serde(rename = "page.edit")]
    PageEdit { page: PageId },
    #[serde(rename = "export.pdf")]
    ExportPdf { path: String, options: ExportOptions },
    #[serde(rename = "session.new")]
    SessionNew,
    #[serde(rename = "session.restore")]
    SessionRestore,
    #[serde(rename = "edit.undo")]
    EditUndo,
    #[serde(rename = "edit.redo")]
    EditRedo,
}

impl Command {
    /// Every command id, for the command-line tool's listing and the fuzz test.
    pub const IDS: &'static [&'static str] = &[
        "import.add",
        "import.discard",
        "crop.set_corner",
        "crop.move_side",
        "crop.bend_side",
        "crop.straighten_side",
        "crop.set_flatten",
        "crop.book",
        "crop.split",
        "crop.set_spine",
        "crop.auto",
        "crop.full_image",
        "crop.apply",
        "page.set_look",
        "page.set_strength",
        "page.rotate",
        "page.add",
        "page.delete",
        "page.move",
        "page.edit",
        "export.pdf",
        "session.new",
        "session.restore",
        "edit.undo",
        "edit.redo",
    ];

    /// Parses `{"id": "...", "params": {...}}`.
    pub fn from_json(text: &str) -> Result<Command, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::Command;

    #[test]
    fn commands_parse_by_id() {
        let c = Command::from_json(r#"{"id":"crop.book","params":{"on":true}}"#).unwrap();
        assert_eq!(c, Command::CropBook { on: true });
        assert!(Command::from_json(r#"{"id":"crop.nope","params":{}}"#).is_err());
        assert_eq!(Command::from_json(r#"{"id":"edit.undo"}"#).unwrap(), Command::EditUndo);
    }
}
