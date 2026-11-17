//! The menu bar (§4.7).
//!
//! Every item runs a command by id. The items that have no command in the contract get one of
//! the interface's own ids, and the app shell runs them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use craft_menu::{Chord, CommandRef, Item, ItemRole, Menu, MenuBar, MenuRole, Node, Platforms};
use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, Screen, ViewState};
use scancraft_core::recipe::{FlattenMode, Look, PageId, PhotoId};

use crate::canvas::dispatch;
use crate::sidebar;
use crate::state::UiState;

/// The interface's own menu ids, for the items the contract has no command for.
pub const ABOUT: &str = "app.about";
pub const SETTINGS: &str = "app.settings";
pub const IMPORT: &str = "file.open";
pub const EXPORT_OPTIONS: &str = "file.export_options";
pub const LABEL_REFERENCE: &str = "file.label_reference";
pub const PREVIOUS_PAGE: &str = "page.previous";
pub const NEXT_PAGE: &str = "page.next";
pub const TOGGLE_PAGES: &str = "view.toggle_pages";
pub const TOGGLE_INSPECTOR: &str = "view.toggle_inspector";
pub const ZOOM_IN: &str = "view.zoom_in";
pub const ZOOM_OUT: &str = "view.zoom_out";
pub const ZOOM_FIT: &str = "view.zoom_fit";
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

/// Builds the menu bar of §4.7.
pub fn build_menu_bar(view: &ViewState) -> MenuBar {
    let page = sidebar::current_page(view);
    let photo = view.crop.as_ref().map(|crop| crop.photo);
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
                Node::Item(item(CommandRef::new("session.new"), "New Scan", "Cmd+Shift+N")),
                Node::Item(item(CommandRef::new(IMPORT), "Add Pages…", "Cmd+O")),
                Node::Item(item(CommandRef::new("export.pdf"), "Export PDF…", "Cmd+E")),
                Node::Item(item(CommandRef::new(EXPORT_OPTIONS), "Export Options…", "Cmd+Alt+E")),
                Node::Item(Item::new(CommandRef::new(LABEL_REFERENCE), "Label for Reference Set…")),
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
                Node::Item(item(page_command("page.delete", page, serde_json::Value::Null), "Delete Page", "⌫")),
            ],
        )
        .role(MenuRole::Edit),
    );

    // Page: what changes the page, in the order of the inspector.
    let split_on = view.crop.as_ref().is_some_and(|crop| crop.outlines.len() < 2 && crop.outlines.first().is_some_and(|o| o.half.is_none()));
    let (look, strength) = view.enhance.as_ref().map_or((Look::ColorDocument, 60), |e| (e.look, e.strength));
    let mut page_items: Vec<Node> = Vec::new();
    page_items.push(Node::Item(item(page_command("page.open_crop", page, serde_json::Value::Null), "Crop", "Cmd+K")));
    page_items.push(Node::Item(item(page_command("crop.auto", crop_page(view), serde_json::Value::Null), "Reset to Detected Edges", "Cmd+D")));
    page_items.push(Node::Item(item(page_command("crop.full_image", crop_page(view), serde_json::Value::Null), "Use Whole Photo", "Cmd+Shift+D")));
    page_items.push(Node::Submenu {
        label: "Flatten".to_owned(),
        children: vec![
            Node::Item(Item::new(flatten_command(crop_page(view), FlattenMode::Auto), "Automatic").radio(true)),
            Node::Item(Item::new(flatten_command(crop_page(view), FlattenMode::On), "Always").radio(true)),
            Node::Item(Item::new(flatten_command(crop_page(view), FlattenMode::Off), "Never").radio(true)),
        ],
        role: None,
        platforms: Platforms::ALL,
    });
    // The item turns the setting over, so it carries the state it is going to: without the
    // parameter `command_of` found no command, and the item and its shortcut did nothing.
    page_items.push(Node::Item(item(
        match photo {
            Some(photo) => CommandRef::with_params("crop.split", serde_json::json!({ "photo": photo.0, "on": split_on })),
            None => CommandRef::new("crop.split"),
        },
        "Split into Two Pages",
        "Cmd+Alt+B",
    )));
    page_items.push(Node::Item(item(page_command("page.rotate", page, serde_json::json!({ "quarter_turns": -1 })), "Rotate Left", "Cmd+L")));
    page_items.push(Node::Item(item(page_command("page.rotate", page, serde_json::json!({ "quarter_turns": 1 })), "Rotate Right", "Cmd+R")));
    let filter_page = view.enhance.as_ref().map(|e| e.page);
    page_items.push(Node::Submenu {
        label: "Filter".to_owned(),
        children: vec![
            Node::Item(Item::new(look_command(filter_page, Look::Original), "Original").radio(true)),
            Node::Item(Item::new(look_command(filter_page, Look::BlackWhite), "Black & White").radio(true)),
            Node::Item(Item::new(look_command(filter_page, Look::Grayscale), "Grayscale").radio(true)),
            Node::Item(Item::new(look_command(filter_page, Look::ColorDocument), "Color Document").radio(true)),
        ],
        role: None,
        platforms: Platforms::ALL,
    });
    page_items.push(Node::Item(item(
        CommandRef::with_params("page.set_look_all", serde_json::json!({ "look": look_name_of(look), "strength": strength })),
        "Apply Filter to All Pages",
        "Cmd+Shift+A",
    )));
    bar.menus.push(Menu::new("Page", page_items));

    // View.
    bar.menus.push(
        Menu::new(
            "View",
            vec![
                Node::Item(item(CommandRef::new(ZOOM_IN), "Zoom In", "Cmd+=")),
                Node::Item(item(CommandRef::new(ZOOM_OUT), "Zoom Out", "Cmd+-")),
                Node::Item(item(CommandRef::new(ZOOM_FIT), "Zoom to Fit", "Cmd+0")),
                Node::Item(item(CommandRef::new(TOGGLE_PAGES), "Show Pages", "Cmd+Alt+T")),
                Node::Item(item(CommandRef::new(TOGGLE_INSPECTOR), "Show Inspector", "Cmd+Alt+I")),
                Node::Item(item(CommandRef::new(PREVIOUS_PAGE), "Previous Page", "Cmd+Left")),
                Node::Item(item(CommandRef::new(NEXT_PAGE), "Next Page", "Cmd+Right")),
            ],
        )
        .role(MenuRole::View),
    );

    bar
}

/// The page the crop items act on: the first page of the photo Crop is open on.
fn crop_page(view: &ViewState) -> Option<PageId> {
    view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.page))
}

/// A filter's name as the command carries it.
fn look_name_of(look: Look) -> &'static str {
    match look {
        Look::Original => "Original",
        Look::BlackWhite => "BlackWhite",
        Look::Grayscale => "Grayscale",
        Look::ColorDocument => "ColorDocument",
    }
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
    sidebar::current_page(view).unwrap_or(PageId(0)).hash(&mut hasher);
    view.enhance.as_ref().map(|enhance| (enhance.look, enhance.strength)).unwrap_or((Look::Original, 0)).hash(&mut hasher);
    let flatten = view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.flatten)).unwrap_or(FlattenMode::Auto);
    match flatten {
        FlattenMode::Auto => 0u8.hash(&mut hasher),
        FlattenMode::On => 1u8.hash(&mut hasher),
        FlattenMode::Off => 2u8.hash(&mut hasher),
    }
    view.crop.as_ref().map(|crop| crop.outlines.len()).unwrap_or(0).hash(&mut hasher);
    view.tray.len().hash(&mut hasher);
    view.can_undo.hash(&mut hasher);
    view.can_redo.hash(&mut hasher);
    view.crop.as_ref().map(|crop| crop.auto_detect).unwrap_or(false).hash(&mut hasher);
    // What decides whether an item is greyed out.
    view.crop.is_some().hash(&mut hasher);
    view.enhance.is_some().hash(&mut hasher);
    sidebar::neighbour_page(view, false).is_some().hash(&mut hasher);
    sidebar::neighbour_page(view, true).is_some().hash(&mut hasher);
    hasher.finish()
}

/// Whether a menu item can act on this view. An item that cannot is greyed out: a menu item that
/// is live and does nothing teaches that the menu cannot be trusted.
pub fn item_enabled(id: &str, view: &ViewState) -> bool {
    let has_pages = !view.tray.is_empty();
    let on_crop = view.crop.is_some();
    let on_enhance = view.enhance.is_some();
    match id {
        "edit.undo" => view.can_undo,
        "edit.redo" => view.can_redo,
        "export.pdf" | EXPORT_OPTIONS | "page.set_look_all" => has_pages,
        "page.delete" | "page.rotate" => sidebar::current_page(view).is_some(),
        "crop.auto" => view.crop.as_ref().is_some_and(|crop| !crop.auto_detect),
        "crop.full_image" | "crop.split" | "crop.set_flatten" | LABEL_REFERENCE => on_crop,
        "page.open_crop" | "page.set_look" => on_enhance,
        PREVIOUS_PAGE => sidebar::neighbour_page(view, false).is_some(),
        NEXT_PAGE => sidebar::neighbour_page(view, true).is_some(),
        TOGGLE_PAGES | TOGGLE_INSPECTOR | ZOOM_IN | ZOOM_OUT | ZOOM_FIT => has_pages && (on_crop || on_enhance || id == TOGGLE_PAGES),
        _ => true,
    }
}

/// Zooms the canvas that is showing about its middle, by `factor`; `None` fits the whole page.
fn zoom(state: &mut UiState, view: &ViewState, factor: Option<f32>) {
    let current = if view.crop.is_some() { &mut state.canvas_view } else { &mut state.enhance_canvas_view };
    *current = match factor {
        Some(factor) => current.zoomed_about_middle(factor),
        None => crate::state::CanvasView::default(),
    };
}

/// The factor a Zoom In or Zoom Out item zooms by.
const ZOOM_STEP: f32 = 1.25;

/// Runs a menu item: the interface's own items, and the commands of the rest.
pub fn handle_menu_command(command: &CommandRef, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    match command.id.as_str() {
        ABOUT => state.about_open = true,
        SETTINGS => state.settings_open = true,
        IMPORT => state.actions.push(crate::state::UiAction::ImportFiles),
        // The options hang from the Export button, which exists only while there are pages: with
        // none, setting the flag drew nothing and the menus took a window to be open.
        EXPORT_OPTIONS => state.export_popover = !state.export_popover && !view.tray.is_empty(),
        LABEL_REFERENCE => state.label_open = true,
        TOGGLE_PAGES => state.sidebar_hidden = !state.sidebar_hidden,
        TOGGLE_INSPECTOR => state.inspector_hidden = !state.inspector_hidden,
        ZOOM_IN => zoom(state, view, Some(ZOOM_STEP)),
        ZOOM_OUT => zoom(state, view, Some(1.0 / ZOOM_STEP)),
        ZOOM_FIT => zoom(state, view, None),
        PREVIOUS_PAGE | NEXT_PAGE => {
            if let Some(page) = sidebar::neighbour_page(view, command.id == NEXT_PAGE) {
                sidebar::open_page(state, view, engine, page);
            }
        }
        "export.pdf" => state.start_export(),
        "page.delete" => {
            if let Some(page) = sidebar::current_page(view)
                && let Some(number) = view.tray.iter().find(|item| item.page == page).map(|item| item.number)
            {
                sidebar::delete_page(state, engine, page, number);
            }
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
