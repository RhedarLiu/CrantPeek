//! Framework-agnostic runtime shared by Crant Peek's UI layers.
//!
//! These modules hold everything the shell should not have to reimplement:
//! config storage, keychain access, the single-instance lock, OS permission
//! prompts, the localisation contract, the double-tap Ctrl hook, selection
//! reading and local OCR. None of them reference a GUI toolkit, so the layer
//! stays independent of whichever windowing library the shell uses.
//!
//! `peek-core` / `peek-network` / `peek-dict` remain untouched by the UI
//! migration; this crate is the seam between them and whichever UI is in use.

pub mod action;
pub mod capture;
pub mod i18n;
pub mod instance;
pub mod ocr;
pub mod permissions;
pub mod prefs;
pub mod selection;
pub mod store;

pub mod updates;
