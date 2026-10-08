//! Events the tray, the global hotkeys and the double-tap Ctrl hook produce.
//!
//! Neutral data, so both the egui shell and the GPUI shell can consume the same
//! platform code.

#[derive(Debug)]
pub enum Action {
    /// Summon an empty panel: never imports the selection or the clipboard.
    Blank,
    /// Summon the panel with the current selection.
    Selection(String),
    /// Start the screenshot translation flow.
    Screenshot,
    /// Text recognised from a screenshot region, ready to be queried.
    Recognized(String),
    Settings,
    TogglePause,
    Quit,
}
