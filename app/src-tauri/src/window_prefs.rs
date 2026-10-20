//! The "Window & tray" settings section: how files from other apps open, what
//! closing the last window does, desktop notifications and starting in the
//! tray. Stored in `settings.json` by [`crate::settings`]; the live values are
//! kept in [`SharedPrefs`] for the window, tray and notification code.

use imt_core::settings::{Configurable, EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

pub const WINDOW_SECTION: &str = "window";

/// Where files opened from another app (second launch, Finder, Explorer) go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenFilesIn {
    #[default]
    NewWindow,
    CurrentWindow,
    Ask,
}

/// What closing the last visible window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnClose {
    #[default]
    Ask,
    Quit,
    Tray,
}

/// Which desktop notifications are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationLevel {
    Off,
    #[default]
    Alerts,
    All,
}

macro_rules! wire_enum {
    ($t:ty { $($v:ident => $s:literal),+ $(,)? }) => {
        impl $t {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$v => $s),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some(Self::$v),)+ _ => None }
            }
        }
    };
}

wire_enum!(OpenFilesIn { NewWindow => "newWindow", CurrentWindow => "currentWindow", Ask => "ask" });
wire_enum!(OnClose { Ask => "ask", Quit => "quit", Tray => "tray" });
wire_enum!(NotificationLevel { Off => "off", Alerts => "alerts", All => "all" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowPrefs {
    pub open_files_in: OpenFilesIn,
    pub on_close: OnClose,
    pub notifications: NotificationLevel,
    pub start_minimized: bool,
}

impl WindowPrefs {
    /// From validated values (missing or unknown values fall back to defaults).
    pub fn from_values(values: &ValidValues) -> WindowPrefs {
        WindowPrefs {
            open_files_in: values.text("openFilesIn").and_then(OpenFilesIn::parse).unwrap_or_default(),
            on_close: values.text("onClose").and_then(OnClose::parse).unwrap_or_default(),
            notifications: values.text("notifications").and_then(NotificationLevel::parse).unwrap_or_default(),
            start_minimized: values.bool("startMinimized").unwrap_or(false),
        }
    }

    /// The section's values as stored.
    pub fn to_json(self) -> Value {
        json!({
            "openFilesIn": self.open_files_in.as_str(),
            "onClose": self.on_close.as_str(),
            "notifications": self.notifications.as_str(),
            "startMinimized": self.start_minimized,
        })
    }
}

/// The values in effect; managed as Tauri state and updated when the section is applied.
#[derive(Clone, Default)]
pub struct SharedPrefs(Arc<RwLock<WindowPrefs>>);

impl SharedPrefs {
    pub fn get(&self) -> WindowPrefs {
        *crate::sync::read(&self.0)
    }

    fn set(&self, prefs: WindowPrefs) {
        *crate::sync::write(&self.0) = prefs;
    }
}

/// The settings adapter registered with the settings service.
pub struct WindowSection {
    pub prefs: SharedPrefs,
}

fn options(list: &[(&str, &str)]) -> FieldKind {
    FieldKind::Enum {
        options: list.iter().map(|(v, l)| EnumOption::new(*v, *l)).collect(),
    }
}

impl Configurable for WindowSection {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new(WINDOW_SECTION, "Window & tray", SectionOwner::App)
            .describe("Windows, the system tray icon and desktop notifications.")
            .field(
                SettingField::new(
                    "openFilesIn",
                    "Open files from other apps in",
                    options(&[
                        ("newWindow", "A new window"),
                        ("currentWindow", "The current window"),
                        ("ask", "Ask each time"),
                    ]),
                    OpenFilesIn::default().as_str(),
                )
                .describe(
                    "Files opened from Finder or Explorer, or by starting IMT again while it runs. An empty window is always reused.",
                ),
            )
            .field(
                SettingField::new(
                    "onClose",
                    "When the last window is closed",
                    options(&[("ask", "Ask"), ("quit", "Quit IMT"), ("tray", "Keep running in the tray")]),
                    OnClose::default().as_str(),
                )
                .describe("In the tray, live tail, the process monitor and downloads keep running."),
            )
            .field(
                SettingField::new(
                    "notifications",
                    "Desktop notifications",
                    options(&[("off", "Off"), ("alerts", "Alerts only"), ("all", "Alerts and finished work")]),
                    NotificationLevel::default().as_str(),
                )
                .describe(
                    "Alerts: live tail alerts while a window is hidden, and a crash detected at start. \
                     Finished work: imports in background windows and completed downloads.",
                ),
            )
            .field(
                SettingField::new("startMinimized", "Start in the system tray", FieldKind::Bool, false)
                    .describe("IMT starts with its window hidden; open it from the tray icon."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        self.prefs.set(WindowPrefs::from_values(values));
        Ok(())
    }
}

/// Registers the section with the settings service and applies its stored values.
pub fn register<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> anyhow::Result<()> {
    use tauri::Manager;
    let prefs = app
        .try_state::<SharedPrefs>()
        .map(|p| p.inner().clone())
        .ok_or_else(|| anyhow::anyhow!("the window preferences are not managed"))?;
    crate::settings::register(app, Arc::new(WindowSection { prefs }), crate::settings::SectionGroup::App)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::settings::{sanitize, validate};

    #[test]
    fn section_round_trips_and_defaults() {
        let prefs = SharedPrefs::default();
        let section = WindowSection { prefs: prefs.clone() };
        let schema = section.settings_section();
        schema.check().unwrap();
        // Defaults: new window, ask, alerts, not minimized.
        let (defaults, errors) = sanitize(&schema, &Value::Null);
        assert!(errors.is_empty());
        section.apply_settings(&defaults).unwrap();
        assert_eq!(prefs.get(), WindowPrefs::default());
        assert_eq!(defaults.to_json(), WindowPrefs::default().to_json());

        let custom = WindowPrefs {
            open_files_in: OpenFilesIn::Ask,
            on_close: OnClose::Tray,
            notifications: NotificationLevel::All,
            start_minimized: true,
        };
        let values = validate(&schema, &custom.to_json()).unwrap();
        section.apply_settings(&values).unwrap();
        assert_eq!(prefs.get(), custom);
        for (s, v) in [("currentWindow", OpenFilesIn::CurrentWindow), ("newWindow", OpenFilesIn::NewWindow)] {
            assert_eq!(OpenFilesIn::parse(s), Some(v));
        }
        assert_eq!(OnClose::parse("quit"), Some(OnClose::Quit));
        assert_eq!(NotificationLevel::parse("off"), Some(NotificationLevel::Off));
        assert_eq!(NotificationLevel::parse("loud"), None);
        assert!(validate(&schema, &json!({ "onClose": "explode" })).is_err());
    }
}
