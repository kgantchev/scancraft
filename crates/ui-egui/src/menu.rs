//! The menu bar (§4.7).
//!
//! Every item runs a command by id. The items that have no command in the contract get one of
//! the interface's own ids, and the app shell runs them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use craft_menu::{Chord, CommandRef, Item, ItemRole, Menu, MenuBar, MenuRole, Node, Platforms};
use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, Screen, ViewState};
use scancraft_core::geom::Side;
use scancraft_core::recipe::{FlattenMode, Look, PageId, PhotoId};

use crate::canvas::dispatch;
use crate::state::{Selection, UiState};

/// The interface's own menu ids, for the items the contract has no command for.
pub const ABOUT: &str = "app.about";
pub const SETTINGS: &str = "app.settings";
pub const IMPORT: &str = "file.open";
pub const EXPORT_OPTIONS: &str = "file.export_options";
pub const PREVIOUS_PHOTO: &str = "page.previous";
pub const NEXT_PHOTO: &str = "page.next";
pub const TOGGLE_TRAY: &str = "view.toggle_tray";
/// The command of the app's own Quit item, which `craft-quit` runs.
pub const QUIT: &str = craft_quit::QUIT_MENU_ID;

/// An item with its shortcut, when the shortcut text is a chord.
fn item(command: CommandRef, label: &str, shortcut: &str) -> Item {
    let mut item = Item::new(command, label);
    if let Ok(chord) = Chord::parse(shortcut) {
        item.shortcut = Some(chord);
    }
    item
}

/// The command of a Flatten item, with the page it applies to.
fn flatten_command(page: Option<PageId>, mode: FlattenMode) -> CommandRef {
    let mode = match mode {
        FlattenMode::Auto => "Auto",
        FlattenMode::On => "On",
        FlattenMode::Off => "Off",
    };
    match page {
        Some(page) => CommandRef::with_params("crop.set_flatten", serde_json::json!({ "page": page.0, "mode": mode })),
        None => CommandRef::new("crop.set_flatten"),
    }
}

/// The command of a Look item, with the page it applies to.
fn look_command(page: Option<PageId>, look: Look) -> CommandRef {
    let look = match look {
        Look::Original => "Original",
        Look::BlackWhite => "BlackWhite",
        Look::Grayscale => "Grayscale",
        Look::ColorDocument => "ColorDocument",
    };
    match page {
        Some(page) => CommandRef::with_params("page.set_look", serde_json::json!({ "page": page.0, "look": look })),
        None => CommandRef::new("page.set_look"),
    }
}

/// The command of an item that acts on a page.
fn page_command(id: &str, page: Option<PageId>, extra: serde_json::Value) -> CommandRef {
    match page {
        Some(page) => {
            let mut params = serde_json::Map::new();
            params.insert("page".to_owned(), serde_json::json!(page.0));
            if let serde_json::Value::Object(extra) = extra {
                for (key, value) in extra {
                    params.insert(key, value);
                }
            }
            CommandRef::with_params(id, serde_json::Value::Object(params))
        }
        None => CommandRef::new(id),
    }
}

/// The command of an item that acts on a photo.
fn photo_command(id: &str, photo: Option<PhotoId>) -> CommandRef {
    match photo {
        Some(photo) => CommandRef::with_params(id, serde_json::json!({ "photo": photo.0 })),
        None => CommandRef::new(id),
    }
}

/// Builds the menu bar of §4.7.
pub fn build_menu_bar(view: &ViewState) -> MenuBar {
    let page = view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.page));
    let photo = view.crop.as_ref().map(|crop| crop.photo);
    let enhance_page = view.enhance.as_ref().map(|enhance| enhance.page);
    let mut bar = MenuBar::default();

    // The app menu.
    bar.menus.push(
        Menu::new(
            "ScanCraft",
            vec![
                Node::Item(Item::new(CommandRef::new(ABOUT), "About ScanCraft").role(ItemRole::About)),
                Node::Item(item(CommandRef::new(SETTINGS), "Settings…", "Cmd+,").role(ItemRole::Settings)),
                Node::Item(Item::new(CommandRef::new(QUIT), "Quit ScanCraft").role(ItemRole::Quit)),
            ],
        )
        .role(MenuRole::App),
    );

    // File.
    bar.menus.push(
        Menu::new(
            "File",
            vec![
                Node::Item(item(CommandRef::new("session.new"), "New Session", "Cmd+Shift+N")),
                Node::Item(item(CommandRef::new(IMPORT), "Import…", "Cmd+O")),
                Node::Item(item(CommandRef::new("export.pdf"), "Export to PDF…", "Cmd+E")),
                Node::Item(item(CommandRef::new(EXPORT_OPTIONS), "Export Options…", "Cmd+Alt+E")),
            ],
        )
        .role(MenuRole::File),
    );

    // Edit.
    bar.menus.push(
        Menu::new(
            "Edit",
            vec![
                Node::Item(item(CommandRef::new("edit.undo"), "Undo", "Cmd+Z")),
                Node::Item(item(CommandRef::new("edit.redo"), "Redo", "Cmd+Shift+Z")),
                Node::Item(item(page_command("page.delete", enhance_page.or(page), serde_json::Value::Null), "Delete Page", "⌫")),
            ],
        )
        .role(MenuRole::Edit),
    );

    // Page.
    let mut page_items: Vec<Node> = Vec::new();
    page_items.push(Node::Item(item(page_command("crop.auto", page, serde_json::Value::Null), "Auto-Detect", "Cmd+D")));
    page_items.push(Node::Item(item(page_command("crop.full_image", page, serde_json::Value::Null), "Use Full Image", "Cmd+Shift+D")));
    page_items.push(Node::Submenu {
        label: "Flatten".to_owned(),
        children: vec![
            Node::Item(Item::new(flatten_command(page, FlattenMode::Auto), "Auto").radio(true)),
            Node::Item(Item::new(flatten_command(page, FlattenMode::On), "On").radio(true)),
            Node::Item(Item::new(flatten_command(page, FlattenMode::Off), "Off").radio(true)),
        ],
        role: None,
        platforms: Platforms::ALL,
    });
    page_items.push(Node::Item(Item::new(CommandRef::new("crop.straighten_side"), "Straighten Side")));
    // The item turns the setting over, so it carries the state it is going to: without the
    // parameter `command_of` found no command, and the item and its shortcut did nothing.
    page_items.push(Node::Item(item(
        CommandRef::with_params("crop.book", serde_json::json!({ "on": !view.book_spreads })),
        "Book Spreads",
        "Cmd+Alt+B",
    )));
    page_items.push(Node::Item(item(photo_command("crop.apply", photo), "Correct Perspective", "↩")));
    page_items.push(Node::Item(item(page_command("page.edit", enhance_page.or(page), serde_json::Value::Null), "Back to Crop", "Cmd+[")));
    page_items.push(Node::Item(item(
        page_command("page.rotate", enhance_page.or(page), serde_json::json!({ "quarter_turns": -1 })),
        "Rotate Left",
        "Cmd+L",
    )));
    page_items.push(Node::Item(item(
        page_command("page.rotate", enhance_page.or(page), serde_json::json!({ "quarter_turns": 1 })),
        "Rotate Right",
        "Cmd+R",
    )));
    page_items.push(Node::Submenu {
        label: "Look".to_owned(),
        children: vec![
            Node::Item(Item::new(look_command(enhance_page, Look::Original), "Original").radio(true)),
            Node::Item(Item::new(look_command(enhance_page, Look::BlackWhite), "Black & White").radio(true)),
            Node::Item(Item::new(look_command(enhance_page, Look::Grayscale), "Grayscale").radio(true)),
            Node::Item(Item::new(look_command(enhance_page, Look::ColorDocument), "Color Document").radio(true)),
        ],
        role: None,
        platforms: Platforms::ALL,
    });
    page_items.push(Node::Item(item(page_command("page.add", enhance_page.or(page), serde_json::Value::Null), "Add Page", "⌘↩")));
    page_items.push(Node::Item(item(CommandRef::new(PREVIOUS_PHOTO), "Previous Photo", "Cmd+Left")));
    page_items.push(Node::Item(item(CommandRef::new(NEXT_PHOTO), "Next Photo", "Cmd+Right")));
    bar.menus.push(Menu::new("Page", page_items));

    // View.
    bar.menus.push(Menu::new("View", vec![Node::Item(item(CommandRef::new(TOGGLE_TRAY), "Show Page Tray", "Cmd+Alt+T"))]).role(MenuRole::View));

    bar
}

/// The hash of everything the menus show: the app rebuilds the bar only when it changes.
pub fn menu_state(view: &ViewState) -> u64 {
    let mut hasher = DefaultHasher::new();
    match view.screen {
        Screen::Import => 0u8.hash(&mut hasher),
        Screen::Crop(_) => 1u8.hash(&mut hasher),
        Screen::Enhance(_) => 2u8.hash(&mut hasher),
        Screen::Export => 3u8.hash(&mut hasher),
    }
    view.crop.as_ref().map(|crop| crop.photo).unwrap_or(PhotoId(0)).hash(&mut hasher);
    view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.page)).unwrap_or(PageId(0)).hash(&mut hasher);
    view.enhance.as_ref().map(|enhance| enhance.page).unwrap_or(PageId(0)).hash(&mut hasher);
    view.enhance.as_ref().map(|enhance| enhance.look).unwrap_or(Look::Original).hash(&mut hasher);
    let flatten = view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.flatten)).unwrap_or(FlattenMode::Auto);
    match flatten {
        FlattenMode::Auto => 0u8.hash(&mut hasher),
        FlattenMode::On => 1u8.hash(&mut hasher),
        FlattenMode::Off => 2u8.hash(&mut hasher),
    }
    view.tray.len().hash(&mut hasher);
    view.can_undo.hash(&mut hasher);
    view.can_redo.hash(&mut hasher);
    view.book_spreads.hash(&mut hasher);
    view.crop.as_ref().map(|crop| crop.auto_detect).unwrap_or(false).hash(&mut hasher);
    hasher.finish()
}

/// Runs a menu item: the interface's own items, and the commands of the rest.
pub fn handle_menu_command(command: &CommandRef, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    match command.id.as_str() {
        ABOUT => state.about_open = true,
        SETTINGS => state.settings_open = true,
        IMPORT => state.actions.push(crate::state::UiAction::ImportFiles),
        EXPORT_OPTIONS => state.export_popover = !state.export_popover,
        TOGGLE_TRAY => state.tray_hidden = !state.tray_hidden,
        PREVIOUS_PHOTO | NEXT_PHOTO => {
            state.notice(state.text.t("Moving between photos needs a command the engine doesn't have yet."), false);
        }
        "export.pdf" => {
            let options = state.export_options.clone();
            state.actions.push(crate::state::UiAction::ExportPdf { options });
        }
        "crop.straighten_side" => {
            let Some(page) = view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.page)) else { return };
            let side = straighten_side(state, view);
            dispatch(state, engine, Command::CropStraightenSide { page, side });
        }
        _ => {
            let Some(command) = command_of(command) else { return };
            dispatch(state, engine, command);
        }
    }
}

/// The command a menu item runs, read back from its id and its parameters.
pub fn command_of(command: &CommandRef) -> Option<Command> {
    let text = serde_json::json!({ "id": command.id, "params": command.params }).to_string();
    Command::from_json(&text).ok()
}

/// The side the Straighten Side item straightens: the selected bend handle's side, or the first
/// side that has a curve.
fn straighten_side(state: &UiState, view: &ViewState) -> Side {
    let outline = view.crop.as_ref().and_then(|crop| crop.outlines.first());
    if let Some(Selection::Bend { side, .. }) = state.selection {
        return side;
    }
    if let Some(outline) = outline {
        for side in Side::ALL {
            if crate::canvas::side_is_curved(outline, side) {
                return side;
            }
        }
    }
    Side::Top
}
