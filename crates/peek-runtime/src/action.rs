//! Events the tray, the global hotkeys and the double-tap Ctrl hook produce.
//!
//! Neutral data, so the shell never touches platform code directly.

use std::sync::mpsc::Sender;

/// Progress of a turn the panel runs on behalf of the screenshot overlay.
///
/// The overlay borrows the panel's query pipeline instead of building a second
/// one: it asks for a translation and receives the streamed result here, so the
/// network stack, configuration and credential handling stay in one place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayEvent {
    /// A localised status line to show in the overlay card.
    Status(String),
    /// A streamed answer chunk.
    Chunk(String),
    /// The turn failed; the overlay raises this as a notification rather than
    /// leaving it in the card.
    Failed(String),
    /// The turn ended, whether it succeeded or not.
    Finished,
}

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
    /// Run a turn for the overlay, reporting progress through `replies` rather
    /// than showing the panel.
    Translate {
        text: String,
        replies: Sender<OverlayEvent>,
    },
    Settings,
    TogglePause,
    Quit,
}
