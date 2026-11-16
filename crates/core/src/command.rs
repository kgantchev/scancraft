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
    /// Puts the same look and strength on every page of the document, as one undo step.
    #[serde(rename = "page.set_look_all")]
    PageSetLookAll { look: Look, strength: u8 },
    #[serde(rename = "page.rotate")]
    PageRotate { page: PageId, quarter_turns: i8 },
    #[serde(rename = "page.add")]
    PageAdd { page: PageId },
    /// The user has looked at a page flagged `PageState::NeedsLook` and it is right as it is: the
    /// flag goes.
    #[serde(rename = "page.mark_reviewed")]
    PageMarkReviewed { page: PageId },
    #[serde(rename = "page.delete")]
    PageDelete { page: PageId },
    #[serde(rename = "page.move")]
    PageMove { from: u32, to: u32 },
    #[serde(rename = "page.edit")]
    PageEdit { page: PageId },
    /// Opens the crop screen (Screen 2) for a page, with that page's corners (the "Back to Crop"
    /// of §4.7 and the tray's "Edit Crop").
    #[serde(rename = "page.open_crop")]
    PageOpenCrop { page: PageId },
    /// Writes the page's corners into the reference set's label file (specification §10.2), as a
    /// photo of the `category` category. The label file is merged with what is already there.
    #[serde(rename = "reference.label")]
    ReferenceLabel { page: PageId, folder: String, category: String },
    #[serde(rename = "export.pdf")]
    ExportPdf { path: String, options: ExportOptions },
    /// Stops the export that is running. The half-written file is removed.
    #[serde(rename = "export.cancel")]
    ExportCancel,
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
        "page.set_look_all",
        "page.rotate",
        "page.add",
        "page.mark_reviewed",
        "page.delete",
        "page.move",
        "page.edit",
        "page.open_crop",
        "reference.label",
        "export.pdf",
        "export.cancel",
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
    use super::{Command, Corner};
    use crate::geom::{Point, Side};
    use crate::recipe::{Look, PageId};

    #[test]
    fn commands_parse_by_id() {
        let c = Command::from_json(r#"{"id":"crop.book","params":{"on":true}}"#).unwrap();
        assert_eq!(c, Command::CropBook { on: true });
        assert!(Command::from_json(r#"{"id":"crop.nope","params":{}}"#).is_err());
        assert_eq!(Command::from_json(r#"{"id":"edit.undo"}"#).unwrap(), Command::EditUndo);
    }

    #[test]
    fn the_two_new_commands_parse_and_are_listed() {
        let crop = Command::from_json(r#"{"id":"page.open_crop","params":{"page":3}}"#).unwrap();
        assert_eq!(crop, Command::PageOpenCrop { page: PageId(3) });
        let label = Command::from_json(r#"{"id":"reference.label","params":{"page":4,"folder":"/tmp/set","category":"dark desk"}}"#).unwrap();
        assert_eq!(label, Command::ReferenceLabel { page: PageId(4), folder: "/tmp/set".to_string(), category: "dark desk".to_string() });
        // Every id is in the list once, and the list has no repeats.
        let mut ids = Command::IDS.to_vec();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "the list of ids has a repeat");
        assert!(Command::IDS.contains(&"page.open_crop"));
        assert!(Command::IDS.contains(&"reference.label"));
    }

    #[test]
    fn the_commands_of_the_review_first_flow_parse_and_are_listed() {
        let all = Command::from_json(r#"{"id":"page.set_look_all","params":{"look":"Grayscale","strength":70}}"#).unwrap();
        assert_eq!(all, Command::PageSetLookAll { look: Look::Grayscale, strength: 70 });
        let reviewed = Command::from_json(r#"{"id":"page.mark_reviewed","params":{"page":5}}"#).unwrap();
        assert_eq!(reviewed, Command::PageMarkReviewed { page: PageId(5) });
        assert_eq!(Command::from_json(r#"{"id":"export.cancel"}"#).unwrap(), Command::ExportCancel);
        for id in ["page.set_look_all", "page.mark_reviewed", "export.cancel"] {
            assert!(Command::IDS.contains(&id), "{id} is not in the list of ids");
        }
        // Missing parameters are refused, not defaulted.
        assert!(Command::from_json(r#"{"id":"page.set_look_all","params":{"look":"Grayscale"}}"#).is_err());
    }

    #[test]
    fn every_command_serialises_to_an_id_in_the_list_and_every_id_has_a_command() {
        use crate::recipe::{ExportOptions, FlattenMode, PhotoId};
        let p = Point::new(1.0, 2.0);
        let samples = vec![
            Command::ImportAdd { paths: vec![] },
            Command::ImportDiscard { item: PhotoId(1) },
            Command::CropSetCorner { page: PageId(1), corner: Corner::TopLeft, x: 0.0, y: 0.0 },
            Command::CropMoveSide { page: PageId(1), side: Side::Top, offset: 1.0 },
            Command::CropBendSide { page: PageId(1), side: Side::Top, handles: [p, p, p] },
            Command::CropStraightenSide { page: PageId(1), side: Side::Top },
            Command::CropSetFlatten { page: PageId(1), mode: FlattenMode::Auto },
            Command::CropBook { on: true },
            Command::CropSplit { photo: PhotoId(1), on: true },
            Command::CropSetSpine { photo: PhotoId(1), top: p, bottom: p },
            Command::CropAuto { page: PageId(1) },
            Command::CropFullImage { page: PageId(1) },
            Command::CropApply { photo: PhotoId(1) },
            Command::PageSetLook { page: PageId(1), look: Look::Original },
            Command::PageSetStrength { page: PageId(1), value: 1 },
            Command::PageSetLookAll { look: Look::Original, strength: 1 },
            Command::PageRotate { page: PageId(1), quarter_turns: 1 },
            Command::PageAdd { page: PageId(1) },
            Command::PageMarkReviewed { page: PageId(1) },
            Command::PageDelete { page: PageId(1) },
            Command::PageMove { from: 0, to: 1 },
            Command::PageEdit { page: PageId(1) },
            Command::PageOpenCrop { page: PageId(1) },
            Command::ReferenceLabel { page: PageId(1), folder: String::new(), category: String::new() },
            Command::ExportPdf { path: String::new(), options: ExportOptions::default() },
            Command::ExportCancel,
            Command::SessionNew,
            Command::SessionRestore,
            Command::EditUndo,
            Command::EditRedo,
        ];
        let mut ids: Vec<String> = samples.iter().map(|c| serde_json::to_value(c).unwrap()["id"].as_str().unwrap().to_owned()).collect();
        for id in &ids {
            assert!(Command::IDS.contains(&id.as_str()), "{id} is not in the list of ids");
        }
        ids.sort_unstable();
        let mut listed: Vec<String> = Command::IDS.iter().map(|id| (*id).to_owned()).collect();
        listed.sort_unstable();
        assert_eq!(ids, listed, "the list of ids and the commands disagree");
    }
}
