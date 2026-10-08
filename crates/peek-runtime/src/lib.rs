//! Framework-agnostic runtime shared by Crant Peek's UI layers.
//!
//! These modules were extracted from `peek-app` so the GPUI shell
//! (`peek-gpui`) can reuse them instead of duplicating config storage,
//! keychain access, the single-instance lock, OS permission prompts, the
//! localisation contract, the double-tap Ctrl hook, selection reading and local
//! OCR. None of them reference a GUI toolkit.
//!
//! `peek-core` / `peek-network` / `peek-dict` remain untouched by the UI
//! migration; this crate is the seam between them and whichever UI is in use.

pub mod action;
pub mod i18n;
pub mod instance;
pub mod ocr;
pub mod permissions;
pub mod prefs;
pub mod selection;
pub mod store;
