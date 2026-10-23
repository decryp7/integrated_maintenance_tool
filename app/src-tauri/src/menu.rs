//! The macOS native menu bar, built from the UI's menu model.
//!
//! The UI owns the menus (one model from its registered views and commands, also behind its in-app
//! menu bar on Windows and Linux). On macOS it sends that model with `set_app_menu`; this module
//! validates it ([`parse`]), turns it into a native menu spec with the standard macOS items
//! ([`native_spec`]: app menu with Services/Hide and the model's `app.quit` last, Edit with Undo..Select All wired to the
//! webview, Window with Minimize/Zoom) and applies it, rebuilding only when the structure changed
//! and otherwise updating check marks and enabled states in place ([`diff`]). A click is sent to
//! the focused window (a pop-out's host) as `debrief://menu` with the item id ([`route`]).
//!
//! On other platforms `set_app_menu` validates and does nothing: the UI shows its own menu bar.
//!
//! Command: `set_app_menu`. Event: `debrief://menu` (to the focused window only).

use crate::CmdResult;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

/// Event carrying the id of the chosen menu item to the focused window.
pub const MENU_EVENT: &str = "debrief://menu";
const MAX_MENUS: usize = 16;
const MAX_ENTRIES: usize = 1000;
const MAX_DEPTH: usize = 4;
const MAX_ID: usize = 128;
const MAX_LABEL: usize = 120;
const MAX_KEYBINDING: usize = 40;

// ---- wire DTOs (mirrors app/src/app/shell/menu/native-menu.types.ts) ----

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuModelDto {
    pub menus: Vec<TopMenuDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopMenuDto {
    pub id: String,
    pub label: String,
    pub items: Vec<EntryDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum EntryDto {
    #[serde(rename_all = "camelCase")]
    Item {
        id: String,
        label: String,
        keybinding: Option<String>,
        checked: Option<bool>,
        enabled: bool,
    },
    Separator,
    Header {
        label: String,
    },
    Submenu {
        label: String,
        items: Vec<EntryDto>,
    },
}

// ---- validated model ----

/// A menu item the UI runs: plain, or a toggle with a check mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub label: String,
    /// A Tauri accelerator ("CmdOrCtrl+Shift+O"); `None` when there is no (usable) key binding.
    pub accelerator: Option<String>,
    pub checked: Option<bool>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Item(Item),
    Separator,
    Header(String),
    Submenu(String, Vec<Entry>),
}

/// The model's Quit command: last in the app menu (the system's own Quit would skip the unsaved-work prompt).
pub const QUIT_COMMAND: &str = "app.quit";

/// The role of a top-level menu: the ones macOS gives standard items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuRole {
    /// Debrief ▸ (About, Settings…); merged with Services and Hide; the model's `app.quit` (Quit Debrief, ⌘Q) goes last.
    App,
    /// Merged with Undo, Redo, Cut, Copy, Paste, Select All.
    Edit,
    /// Merged with Minimize and Zoom.
    Window,
    Help,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopMenu {
    pub role: MenuRole,
    pub label: String,
    pub items: Vec<Entry>,
}

/// The validated menu model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppMenu {
    pub menus: Vec<TopMenu>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuError {
    TooManyMenus(usize),
    TooManyEntries,
    TooDeep,
    BadId(String),
    DuplicateId(String),
    BadLabel(String),
}

impl std::fmt::Display for MenuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MenuError::TooManyMenus(n) => write!(f, "the menu bar has {n} menus; at most {MAX_MENUS} are allowed"),
            MenuError::TooManyEntries => write!(f, "the menus have more than {MAX_ENTRIES} entries"),
            MenuError::TooDeep => write!(f, "submenus are nested more than {MAX_DEPTH} levels deep"),
            MenuError::BadId(id) => write!(f, "invalid menu item id {:?}", truncate(id, 40)),
            MenuError::DuplicateId(id) => write!(f, "the menu item id {id:?} is used twice"),
            MenuError::BadLabel(l) => write!(f, "invalid menu label {:?}", truncate(l, 40)),
        }
    }
}

impl std::error::Error for MenuError {}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
}

fn label(text: String) -> Result<String, MenuError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_LABEL || trimmed.chars().any(char::is_control) {
        return Err(MenuError::BadLabel(text));
    }
    Ok(trimmed.to_string())
}

/// Turns a UI key binding ("Mod+Shift+O", "Alt+Left", "F1", "Ctrl+Alt+1") into a Tauri
/// accelerator ("CmdOrCtrl+Shift+O"). `None` for a binding a menu cannot show.
pub fn accelerator(binding: &str) -> Option<String> {
    if binding.is_empty() || binding.len() > MAX_KEYBINDING {
        return None;
    }
    let parts: Vec<&str> = binding.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    let mut out = Vec::new();
    for m in mods {
        out.push(match m.to_ascii_lowercase().as_str() {
            "mod" => "CmdOrCtrl",
            "ctrl" | "control" => "Ctrl",
            "alt" | "option" => "Alt",
            "shift" => "Shift",
            "meta" | "cmd" => "Super",
            _ => return None,
        });
    }
    let key = key_name(key)?;
    out.push(&key);
    Some(out.join("+"))
}

fn key_name(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    let named = match lower.as_str() {
        "-" => "Minus",
        "," => "Comma",
        "." => "Period",
        "/" => "Slash",
        ";" => "Semicolon",
        "=" => "Equal",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "\\" => "Backslash",
        "'" => "Quote",
        "`" => "Backquote",
        "enter" | "return" => "Enter",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "space" => "Space",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "left" | "arrowleft" => "ArrowLeft",
        "right" | "arrowright" => "ArrowRight",
        "up" | "arrowup" => "ArrowUp",
        "down" | "arrowdown" => "ArrowDown",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase().to_string()),
        _ => {
            let n: u8 = lower.strip_prefix('f')?.parse().ok()?;
            (1..=24).contains(&n).then(|| format!("F{n}"))
        }
    }
}

/// Validates the UI's menu model (sizes, ids, labels) and converts key bindings to accelerators.
pub fn parse(dto: MenuModelDto) -> Result<AppMenu, MenuError> {
    if dto.menus.len() > MAX_MENUS {
        return Err(MenuError::TooManyMenus(dto.menus.len()));
    }
    let mut seen = HashSet::new();
    let mut count = 0usize;
    let mut menus = Vec::with_capacity(dto.menus.len());
    for m in dto.menus {
        let role = match m.id.as_str() {
            "app" => MenuRole::App,
            "edit" => MenuRole::Edit,
            "window" => MenuRole::Window,
            "help" => MenuRole::Help,
            id if valid_id(id) => MenuRole::Other,
            id => return Err(MenuError::BadId(id.to_string())),
        };
        menus.push(TopMenu {
            role,
            label: label(m.label)?,
            items: entries(m.items, 1, &mut seen, &mut count)?,
        });
    }
    Ok(AppMenu { menus })
}

fn entries(dtos: Vec<EntryDto>, depth: usize, seen: &mut HashSet<String>, count: &mut usize) -> Result<Vec<Entry>, MenuError> {
    if depth > MAX_DEPTH {
        return Err(MenuError::TooDeep);
    }
    let mut out = Vec::with_capacity(dtos.len());
    for dto in dtos {
        *count += 1;
        if *count > MAX_ENTRIES {
            return Err(MenuError::TooManyEntries);
        }
        out.push(match dto {
            EntryDto::Item {
                id,
                label: text,
                keybinding,
                checked,
                enabled,
            } => {
                if !valid_id(&id) {
                    return Err(MenuError::BadId(id));
                }
                if !seen.insert(id.clone()) {
                    return Err(MenuError::DuplicateId(id));
                }
                Entry::Item(Item {
                    id,
                    label: label(text)?,
                    // A binding a menu cannot show still works in the page; the item just has no shortcut.
                    accelerator: keybinding.as_deref().and_then(accelerator),
                    checked,
                    enabled,
                })
            }
            EntryDto::Separator => Entry::Separator,
            EntryDto::Header { label: text } => Entry::Header(label(text)?),
            EntryDto::Submenu { label: text, items } => Entry::Submenu(label(text)?, entries(items, depth + 1, seen, count)?),
        });
    }
    Ok(out)
}

// ---- native menu spec (pure) ----

/// Standard macOS items, implemented by the system (the Edit ones act on the webview's text fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predefined {
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Native {
    Item(Item),
    /// A disabled caption.
    Header(String),
    Separator,
    Predefined(Predefined),
    Submenu {
        label: String,
        role: MenuRole,
        items: Vec<Native>,
    },
}

fn natives(entries: &[Entry]) -> Vec<Native> {
    entries
        .iter()
        .map(|e| match e {
            Entry::Item(i) => Native::Item(i.clone()),
            Entry::Separator => Native::Separator,
            Entry::Header(h) => Native::Header(h.clone()),
            Entry::Submenu(label, items) => Native::Submenu {
                label: label.clone(),
                role: MenuRole::Other,
                items: natives(items),
            },
        })
        .collect()
}

/// `standard`, then the model's items after a separator (when there are any).
fn merged(standard: Vec<Native>, model: Option<&TopMenu>) -> Vec<Native> {
    let mut items = standard;
    if let Some(m) = model.filter(|m| !m.items.is_empty()) {
        items.push(Native::Separator);
        items.extend(natives(&m.items));
    }
    items
}

/// The native menu bar for a model: the app menu first, Edit and Window always present (with
/// their standard items), the model's menus in its order. Pure.
pub fn native_spec(model: &AppMenu) -> Vec<Native> {
    use Predefined as P;
    let find = |role| model.menus.iter().find(|m| m.role == role);
    let mut app_items: Vec<Native> = find(MenuRole::App).map(|m| natives(&m.items)).unwrap_or_default();
    if !app_items.is_empty() {
        app_items.push(Native::Separator);
    }
    app_items.extend([
        Native::Predefined(P::Services),
        Native::Separator,
        Native::Predefined(P::Hide),
        Native::Predefined(P::HideOthers),
        Native::Predefined(P::ShowAll),
    ]);
    // Quit is the model's own `app.quit` (it asks about unsaved work first), placed last.
    if let Some(quit) = app_items.iter().position(|n| matches!(n, Native::Item(i) if i.id == QUIT_COMMAND)) {
        let item = app_items.remove(quit);
        app_items.extend([Native::Separator, item]);
    }
    let edit = || Native::Submenu {
        label: find(MenuRole::Edit).map_or_else(|| "Edit".to_string(), |m| m.label.clone()),
        role: MenuRole::Edit,
        items: merged(
            vec![
                Native::Predefined(P::Undo),
                Native::Predefined(P::Redo),
                Native::Separator,
                Native::Predefined(P::Cut),
                Native::Predefined(P::Copy),
                Native::Predefined(P::Paste),
                Native::Predefined(P::SelectAll),
            ],
            find(MenuRole::Edit),
        ),
    };
    let window = || Native::Submenu {
        label: find(MenuRole::Window).map_or_else(|| "Window".to_string(), |m| m.label.clone()),
        role: MenuRole::Window,
        items: merged(
            vec![Native::Predefined(P::Minimize), Native::Predefined(P::Zoom)],
            find(MenuRole::Window),
        ),
    };
    let mut bar = vec![Native::Submenu {
        label: "Debrief".into(),
        role: MenuRole::App,
        items: app_items,
    }];
    // Without its own Edit menu, the model gets the standard one right after its first menu (File).
    let (mut has_edit, mut has_window) = (false, false);
    let model_has_edit = find(MenuRole::Edit).is_some();
    for m in &model.menus {
        match m.role {
            MenuRole::App => {}
            MenuRole::Edit => {
                has_edit = true;
                bar.push(edit());
            }
            MenuRole::Window => {
                has_window = true;
                bar.push(window());
            }
            role => {
                bar.push(Native::Submenu {
                    label: m.label.clone(),
                    role,
                    items: natives(&m.items),
                });
                if !model_has_edit && !has_edit && bar.len() == 2 {
                    has_edit = true;
                    bar.push(edit());
                }
            }
        }
    }
    if !has_edit {
        bar.push(edit());
    }
    if !has_window {
        let help = bar.iter().position(|n| matches!(n, Native::Submenu { role: MenuRole::Help, .. }));
        bar.insert(help.unwrap_or(bar.len()), window());
    }
    bar
}

/// Ids of the items the UI runs, depth first.
pub fn item_ids(spec: &[Native]) -> Vec<&str> {
    let mut out = Vec::new();
    for n in spec {
        match n {
            Native::Item(i) => out.push(i.id.as_str()),
            Native::Submenu { items, .. } => out.extend(item_ids(items)),
            _ => {}
        }
    }
    out
}

/// What applying a new spec over the shown one takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Unchanged,
    /// Same structure: only these items' check marks or enabled states changed.
    States(Vec<Item>),
    Rebuild,
}

/// The spec with item states blanked: two specs with the same shape differ only in states.
fn shape(spec: &[Native]) -> Vec<Native> {
    spec.iter()
        .map(|n| match n {
            Native::Item(i) => Native::Item(Item {
                enabled: true,
                checked: i.checked.map(|_| false),
                ..i.clone()
            }),
            Native::Submenu { label, role, items } => Native::Submenu {
                label: label.clone(),
                role: *role,
                items: shape(items),
            },
            other => other.clone(),
        })
        .collect()
}

fn items_of(spec: &[Native]) -> Vec<&Item> {
    let mut out = Vec::new();
    for n in spec {
        match n {
            Native::Item(i) => out.push(i),
            Native::Submenu { items, .. } => out.extend(items_of(items)),
            _ => {}
        }
    }
    out
}

/// Compares the shown spec with a new one. Pure.
pub fn diff(shown: Option<&[Native]>, new: &[Native]) -> Update {
    let Some(old) = shown else { return Update::Rebuild };
    if old == new {
        return Update::Unchanged;
    }
    if shape(old) != shape(new) {
        return Update::Rebuild;
    }
    let changed = items_of(old)
        .into_iter()
        .zip(items_of(new))
        .filter(|(a, b)| a != b)
        .map(|(_, b)| b.clone())
        .collect();
    Update::States(changed)
}

/// Where a click on item `id` goes: the focused window (a pop-out stands for its host), if the
/// id is one of the shown menu's items (the tray's items arrive here too and are not ours). Pure.
pub fn route(id: &str, shown: &[&str], focused: Option<&str>) -> Option<String> {
    if !shown.contains(&id) {
        return None;
    }
    focused.map(|label| crate::panel_windows::resolve(label).to_string())
}

/// Whether the model sent by a window is applied: the menu bar shows the focused window's menus.
pub fn applies(caller_focused: bool, caller_is_target: bool) -> bool {
    caller_focused || caller_is_target
}

// ---- Tauri glue ----

/// Managed state: the spec the menu bar shows.
#[derive(Default)]
pub struct NativeMenu {
    shown: Mutex<Option<Vec<Native>>>,
}

fn build<R: Runtime>(app: &AppHandle<R>, spec: &[Native]) -> tauri::Result<tauri::menu::Menu<R>> {
    let menu = tauri::menu::Menu::new(app)?;
    for n in spec {
        if let Native::Submenu { label, role, items } = n {
            let sub = submenu(app, label, items)?;
            #[cfg(target_os = "macos")]
            {
                if *role == MenuRole::Window {
                    sub.set_as_windows_menu_for_nsapp()?;
                }
                if *role == MenuRole::Help {
                    sub.set_as_help_menu_for_nsapp()?;
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = role;
            menu.append(&sub)?;
        }
    }
    Ok(menu)
}

fn submenu<R: Runtime>(app: &AppHandle<R>, label: &str, items: &[Native]) -> tauri::Result<tauri::menu::Submenu<R>> {
    use tauri::menu::{CheckMenuItem, MenuItem, PredefinedMenuItem, Submenu};
    let sub = Submenu::new(app, label, true)?;
    for n in items {
        match n {
            Native::Item(i) => match i.checked {
                Some(checked) => sub.append(&CheckMenuItem::with_id(
                    app,
                    &i.id,
                    &i.label,
                    i.enabled,
                    checked,
                    i.accelerator.as_deref(),
                )?)?,
                None => sub.append(&MenuItem::with_id(app, &i.id, &i.label, i.enabled, i.accelerator.as_deref())?)?,
            },
            Native::Header(text) => sub.append(&MenuItem::new(app, text, false, None::<&str>)?)?,
            Native::Separator => sub.append(&PredefinedMenuItem::separator(app)?)?,
            Native::Predefined(p) => sub.append(&predefined(app, *p)?)?,
            Native::Submenu { label, items, .. } => sub.append(&submenu(app, label, items)?)?,
        }
    }
    Ok(sub)
}

fn predefined<R: Runtime>(app: &AppHandle<R>, p: Predefined) -> tauri::Result<tauri::menu::PredefinedMenuItem<R>> {
    use tauri::menu::PredefinedMenuItem as M;
    match p {
        Predefined::Services => M::services(app, None),
        Predefined::Hide => M::hide(app, Some("Hide Debrief")),
        Predefined::HideOthers => M::hide_others(app, None),
        Predefined::ShowAll => M::show_all(app, None),
        Predefined::Undo => M::undo(app, None),
        Predefined::Redo => M::redo(app, None),
        Predefined::Cut => M::cut(app, None),
        Predefined::Copy => M::copy(app, None),
        Predefined::Paste => M::paste(app, None),
        Predefined::SelectAll => M::select_all(app, None),
        Predefined::Minimize => M::minimize(app, None),
        Predefined::Zoom => M::maximize(app, Some("Zoom")),
    }
}

/// Finds an item by id anywhere in the menu bar.
fn find<R: Runtime>(items: Vec<tauri::menu::MenuItemKind<R>>, id: &str) -> Option<tauri::menu::MenuItemKind<R>> {
    for kind in items {
        if kind.id().as_ref() == id {
            return Some(kind);
        }
        if let Some(sub) = kind.as_submenu() {
            if let Some(found) = sub.items().ok().and_then(|items| find(items, id)) {
                return Some(found);
            }
        }
    }
    None
}

fn set_states<R: Runtime>(app: &AppHandle<R>, changed: &[Item]) -> tauri::Result<bool> {
    let Some(menu) = app.menu() else { return Ok(false) };
    let items = menu.items()?;
    for item in changed {
        match find(items.clone(), &item.id) {
            Some(tauri::menu::MenuItemKind::Check(c)) => {
                c.set_enabled(item.enabled)?;
                c.set_checked(item.checked.unwrap_or(false))?;
            }
            Some(tauri::menu::MenuItemKind::MenuItem(m)) => m.set_enabled(item.enabled)?,
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// Shows `spec` in the menu bar: nothing when unchanged, states in place when only they changed, else a rebuild.
pub fn apply<R: Runtime>(app: &AppHandle<R>, spec: Vec<Native>) -> tauri::Result<Update> {
    let state = app.state::<NativeMenu>();
    let mut shown = crate::sync::lock(&state.shown);
    let update = diff(shown.as_deref(), &spec);
    let rebuild = match &update {
        Update::Unchanged => false,
        Update::States(changed) => !set_states(app, changed)?,
        Update::Rebuild => true,
    };
    if rebuild {
        app.set_menu(build(app, &spec)?)?;
        log::info!(Menus = spec.len(), Items = item_ids(&spec).len(); "Native menu bar built with {} menus", spec.len());
    }
    *shown = Some(spec);
    Ok(update)
}

/// Routes clicks on the menu bar to the focused window. On macOS, also replaces Tauri's default
/// menu with the standard one until the UI sends its model.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    app.manage(NativeMenu::default());
    app.on_menu_event(|app, event| {
        let id = event.id().as_ref();
        let target = {
            let state = app.state::<NativeMenu>();
            let shown = crate::sync::lock(&state.shown);
            let ids = shown.as_deref().map(item_ids).unwrap_or_default();
            route(id, &ids, crate::window_manager::target(app).as_deref())
        };
        if let Some(label) = target {
            use tauri::Emitter;
            // Only that window (not its pop-outs): it runs the command once.
            let _ = app.emit_to(&label, MENU_EVENT, id.to_string());
        }
    });
    if cfg!(target_os = "macos") {
        if let Err(e) = apply(app, native_spec(&AppMenu::default())) {
            log::warn!("could not build the native menu bar: {e}");
        }
    }
}

/// The UI's menus for the native menu bar (macOS). Elsewhere the model is validated and ignored.
#[tauri::command]
pub async fn set_app_menu<R: Runtime>(app: AppHandle<R>, window: tauri::Window<R>, menu: MenuModelDto) -> CmdResult<()> {
    let model = parse(menu).map_err(crate::err)?;
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let label = crate::panel_windows::resolve(window.label()).to_string();
    let is_target = crate::window_manager::target(&app).as_deref() == Some(label.as_str());
    if !applies(window.is_focused().unwrap_or(false), is_target) {
        return Ok(());
    }
    crate::error::lift(apply(&app, native_spec(&model)).map(|_| ()).map_err(crate::err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;
    use serde_json::json;

    fn model(v: serde_json::Value) -> Result<AppMenu, MenuError> {
        parse(serde_json::from_value(v).unwrap())
    }

    fn item(id: &str, label: &str) -> serde_json::Value {
        json!({ "kind": "item", "id": id, "label": label, "keybinding": null, "checked": null, "enabled": true })
    }

    fn sample() -> AppMenu {
        model(json!({ "menus": [
            { "id": "app", "label": "Debrief", "items": [item("help.about", "About Debrief"), { "kind": "separator" }, item("settings.open", "Settings…")] },
            { "id": "file", "label": "File", "items": [
                { "kind": "item", "id": "file.openFiles", "label": "Open files…", "keybinding": "Mod+O", "checked": null, "enabled": true }
            ] },
            { "id": "edit", "label": "Edit", "items": [item("edit.undo", "Undo filter")] },
            { "id": "view", "label": "View", "items": [
                { "kind": "item", "id": "view:timeline", "label": "Timeline", "keybinding": null, "checked": true, "enabled": true },
                { "kind": "submenu", "label": "Analysis", "items": [
                    { "kind": "item", "id": "view:anomalies", "label": "Anomalies", "keybinding": null, "checked": false, "enabled": true }
                ] },
                { "kind": "header", "label": "Windows registry" }
            ] },
            { "id": "window", "label": "Window", "items": [item("window.layout.reset", "Reset window layout")] },
            { "id": "help", "label": "Help", "items": [item("help.shortcuts", "Keyboard shortcuts")] }
        ] }))
        .unwrap()
    }

    #[test]
    fn accelerators_follow_the_ui_bindings() {
        assert_eq!(accelerator("Mod+Shift+O").as_deref(), Some("CmdOrCtrl+Shift+O"));
        assert_eq!(accelerator("Mod+,").as_deref(), Some("CmdOrCtrl+Comma"));
        assert_eq!(accelerator("Alt+-").as_deref(), Some("Alt+Minus"));
        assert_eq!(accelerator("Ctrl+Alt+1").as_deref(), Some("Ctrl+Alt+1"));
        assert_eq!(accelerator("Shift+Alt+Enter").as_deref(), Some("Shift+Alt+Enter"));
        assert_eq!(accelerator("Alt+Left").as_deref(), Some("Alt+ArrowLeft"));
        assert_eq!(accelerator("Mod+Shift+F12").as_deref(), Some("CmdOrCtrl+Shift+F12"));
        assert_eq!(accelerator("F1").as_deref(), Some("F1"));
        assert_eq!(accelerator("Meta+k").as_deref(), Some("Super+K"));
        assert_eq!(accelerator("Mod+/").as_deref(), Some("CmdOrCtrl+Slash"));
        for bad in ["", "Mod+", "Hyper+K", "Mod+F25", "Mod+Fx", "Mod+é", &"K+".repeat(30)] {
            assert_eq!(accelerator(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn every_converted_key_name_is_one_tauri_accepts() {
        // Names of muda's accelerator parser (the one Tauri's menus use).
        let keys = [
            "-",
            ",",
            ".",
            "/",
            ";",
            "=",
            "[",
            "]",
            "\\",
            "'",
            "`",
            "Enter",
            "Tab",
            "Esc",
            "Space",
            "Backspace",
            "Delete",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Left",
            "Right",
            "Up",
            "Down",
            "a",
            "7",
            "F24",
        ];
        let known = [
            "Minus",
            "Comma",
            "Period",
            "Slash",
            "Semicolon",
            "Equal",
            "BracketLeft",
            "BracketRight",
            "Backslash",
            "Quote",
            "Backquote",
            "Enter",
            "Tab",
            "Escape",
            "Space",
            "Backspace",
            "Delete",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "ArrowLeft",
            "ArrowRight",
            "ArrowUp",
            "ArrowDown",
            "A",
            "7",
            "F24",
        ];
        for (k, want) in keys.iter().zip(known) {
            assert_eq!(key_name(k).as_deref(), Some(want), "{k}");
        }
    }

    #[test]
    fn parse_validates_ids_labels_and_sizes() {
        let m = sample();
        assert_eq!(m.menus.len(), 6);
        assert_eq!(m.menus[0].role, MenuRole::App);
        let Entry::Item(open) = &m.menus[1].items[0] else { panic!() };
        assert_eq!(open.accelerator.as_deref(), Some("CmdOrCtrl+O"));

        let bad_id = model(json!({ "menus": [{ "id": "file", "label": "File", "items": [item("rm -rf", "x")] }] }));
        assert!(matches!(bad_id, Err(MenuError::BadId(_))));
        let dup = model(json!({ "menus": [{ "id": "file", "label": "File", "items": [item("a", "x"), item("a", "y")] }] }));
        assert_eq!(dup, Err(MenuError::DuplicateId("a".into())));
        let empty_label = model(json!({ "menus": [{ "id": "file", "label": "  ", "items": [] }] }));
        assert!(matches!(empty_label, Err(MenuError::BadLabel(_))));
        let control = model(json!({ "menus": [{ "id": "file", "label": "File", "items": [item("a", "x\u{7}")] }] }));
        assert!(matches!(control, Err(MenuError::BadLabel(_))));
        let long = model(json!({ "menus": [{ "id": "file", "label": "x".repeat(MAX_LABEL + 1), "items": [] }] }));
        assert!(matches!(long, Err(MenuError::BadLabel(_))));
        let bad_top = model(json!({ "menus": [{ "id": "", "label": "File", "items": [] }] }));
        assert!(matches!(bad_top, Err(MenuError::BadId(_))));

        let many = (0..=MAX_MENUS)
            .map(|i| json!({ "id": format!("m{i}"), "label": "M", "items": [] }))
            .collect::<Vec<_>>();
        assert_eq!(model(json!({ "menus": many })), Err(MenuError::TooManyMenus(MAX_MENUS + 1)));
        let entries = (0..=MAX_ENTRIES).map(|i| item(&format!("i{i}"), "I")).collect::<Vec<_>>();
        assert_eq!(
            model(json!({ "menus": [{ "id": "f", "label": "F", "items": entries }] })),
            Err(MenuError::TooManyEntries)
        );
        let mut deep = json!([item("leaf", "Leaf")]);
        for _ in 0..MAX_DEPTH {
            deep = json!([{ "kind": "submenu", "label": "S", "items": deep }]);
        }
        assert_eq!(
            model(json!({ "menus": [{ "id": "f", "label": "F", "items": deep }] })),
            Err(MenuError::TooDeep)
        );
        // Unknown fields are rejected: the contract is exact.
        let extra: Result<MenuModelDto, _> = serde_json::from_value(json!({ "menus": [], "script": "x" }));
        assert!(extra.is_err());
        // A binding a menu cannot show is dropped, not fatal.
        let odd = model(json!({ "menus": [{ "id": "f", "label": "F", "items": [
            { "kind": "item", "id": "a", "label": "A", "keybinding": "Hyper+Q", "checked": null, "enabled": true }
        ] }] }))
        .unwrap();
        let Entry::Item(a) = &odd.menus[0].items[0] else { panic!() };
        assert_eq!(a.accelerator, None);
        for e in [
            MenuError::TooManyMenus(20),
            MenuError::TooManyEntries,
            MenuError::TooDeep,
            MenuError::BadId("x".into()),
            MenuError::DuplicateId("x".into()),
            MenuError::BadLabel("x".into()),
        ] {
            assert!(!e.to_string().is_empty());
        }
    }

    fn labels(spec: &[Native]) -> Vec<&str> {
        spec.iter()
            .filter_map(|n| match n {
                Native::Submenu { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect()
    }

    fn submenu<'a>(spec: &'a [Native], name: &str) -> &'a [Native] {
        spec.iter()
            .find_map(|n| match n {
                Native::Submenu { label, items, .. } if label == name => Some(items.as_slice()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn the_spec_adds_the_standard_macos_items() {
        let spec = native_spec(&sample());
        assert_eq!(labels(&spec), ["Debrief", "File", "Edit", "View", "Window", "Help"]);
        let app = submenu(&spec, "Debrief");
        assert!(matches!(&app[0], Native::Item(i) if i.id == "help.about"));
        assert!(app.contains(&Native::Predefined(Predefined::Services)));
        assert_eq!(
            app.last(),
            Some(&Native::Predefined(Predefined::ShowAll)),
            "no Quit item in the model: none added"
        );
        // The model's Quit (it asks about unsaved work) goes last, after a separator.
        let with_quit = model(json!({ "menus": [
            { "id": "app", "label": "Debrief", "items": [item(QUIT_COMMAND, "Quit Debrief"), item("help.about", "About Debrief")] }
        ] }))
        .unwrap();
        let quit_spec = native_spec(&with_quit);
        let quit_app = submenu(&quit_spec, "Debrief");
        assert!(matches!(quit_app.last(), Some(Native::Item(i)) if i.id == QUIT_COMMAND));
        assert_eq!(quit_app[quit_app.len() - 2], Native::Separator);
        assert!(matches!(&quit_app[0], Native::Item(i) if i.id == "help.about"));
        let edit = submenu(&spec, "Edit");
        assert_eq!(edit[0], Native::Predefined(Predefined::Undo));
        assert!(edit.contains(&Native::Predefined(Predefined::Paste)));
        assert!(matches!(edit.last(), Some(Native::Item(i)) if i.id == "edit.undo"));
        let window = submenu(&spec, "Window");
        assert_eq!(window[0], Native::Predefined(Predefined::Minimize));
        let view = submenu(&spec, "View");
        assert_eq!(labels(view), ["Analysis"]);
        assert_eq!(
            item_ids(&spec),
            [
                "help.about",
                "settings.open",
                "file.openFiles",
                "edit.undo",
                "view:timeline",
                "view:anomalies",
                "window.layout.reset",
                "help.shortcuts"
            ]
        );
        assert!(matches!(
            spec.iter().find(|n| matches!(n, Native::Submenu { label, .. } if label == "Help")),
            Some(Native::Submenu { role: MenuRole::Help, .. })
        ));
    }

    #[test]
    fn an_empty_model_still_has_the_app_edit_and_window_menus() {
        let spec = native_spec(&AppMenu::default());
        assert_eq!(labels(&spec), ["Debrief", "Edit", "Window"]);
        assert!(item_ids(&spec).is_empty());
        // Without its own Edit menu, Edit follows File; Window goes before Help.
        let m = model(json!({ "menus": [
            { "id": "file", "label": "File", "items": [] }, { "id": "view", "label": "View", "items": [] }, { "id": "help", "label": "Help", "items": [] }
        ] }))
        .unwrap();
        assert_eq!(labels(&native_spec(&m)), ["Debrief", "File", "Edit", "View", "Window", "Help"]);
    }

    #[test]
    fn diff_updates_states_in_place_and_rebuilds_on_structure_changes() {
        let spec = native_spec(&sample());
        assert_eq!(diff(None, &spec), Update::Rebuild);
        assert_eq!(diff(Some(&spec), &spec), Update::Unchanged);

        let mut toggled = sample();
        let Entry::Item(timeline) = &mut toggled.menus[3].items[0] else {
            panic!()
        };
        timeline.checked = Some(false);
        let new = native_spec(&toggled);
        let Update::States(changed) = diff(Some(&spec), &new) else {
            panic!("expected a state update")
        };
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].id, "view:timeline");
        assert_eq!(changed[0].checked, Some(false));

        let mut relabelled = sample();
        let Entry::Item(undo) = &mut relabelled.menus[2].items[0] else {
            panic!()
        };
        undo.label = "Undo marker edit".into();
        assert_eq!(diff(Some(&spec), &native_spec(&relabelled)), Update::Rebuild);

        let mut fewer = sample();
        fewer.menus[3].items.pop();
        assert_eq!(diff(Some(&spec), &native_spec(&fewer)), Update::Rebuild);
    }

    #[test]
    fn clicks_go_to_the_focused_window_or_its_host() {
        let ids = ["view:anomalies", "help.about"];
        assert_eq!(route("view:anomalies", &ids, Some("main")).as_deref(), Some("main"));
        assert_eq!(route("help.about", &ids, Some("w-2")).as_deref(), Some("w-2"));
        // A pop-out shares its host's menu bar: the host runs the command.
        assert_eq!(route("view:anomalies", &ids, Some("p-w-2-1")).as_deref(), Some("w-2"));
        // Not ours (the tray's items), or no window.
        assert_eq!(route("quit", &ids, Some("main")), None);
        assert_eq!(route("view:anomalies", &ids, None), None);
        assert!(applies(true, false) && applies(false, true) && !applies(false, false));
    }

    // Native menus can only be created on the macOS main thread, which a test is not on: the build
    // itself is checked in the real app; here the command's validation and the non-macOS path.
    #[test]
    fn set_app_menu_rejects_an_invalid_model() {
        use crate::test_support::{test_app, window};
        let app = test_app(None);
        app.manage(NativeMenu::default());
        let main = window(&app, crate::windows::MAIN);
        let bad: MenuModelDto =
            serde_json::from_value(json!({ "menus": [{ "id": "f", "label": "F", "items": [item("a", "x"), item("a", "y")] }] })).unwrap();
        let err = set_app_menu(app.handle().clone(), main.as_ref().window(), bad).now().unwrap_err();
        assert!(err.contains("used twice"), "{err}");
        #[cfg(not(target_os = "macos"))]
        {
            let good: MenuModelDto =
                serde_json::from_value(json!({ "menus": [{ "id": "f", "label": "F", "items": [item("a", "x")] }] })).unwrap();
            assert_eq!(set_app_menu(app.handle().clone(), main.as_ref().window(), good).now(), Ok(()));
        }
    }
}
