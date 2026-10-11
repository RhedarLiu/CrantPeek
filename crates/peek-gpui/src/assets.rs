//! Keep the component library's default assets plus every icon used by this app.
//! The full Lucide catalog is unnecessary for the application's fixed interface.
use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    AppIcons,
    [
        ArrowDown,
        ArrowLeft,
        ArrowUp,
        BookA,
        BookOpen,
        Bug,
        Cable,
        Code,
        Copy,
        CornerDownLeft,
        ExternalLink,
        Languages,
        Pencil,
        Pin,
        PinOff,
        PlugZap,
        Plus,
        Send,
        Settings,
        Sparkles,
        Square,
        Trash,
        X,
    ]
);

#[derive(Clone, Copy, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_and_component_assets_remain_available() {
        let mut expected = gpui_kit::assets::Assets.list("").unwrap();
        expected.extend(AppIcons.list("").unwrap());
        for path in expected {
            let bytes = AppAssets.load(&path).unwrap().unwrap();
            assert!(
                std::str::from_utf8(&bytes).unwrap().contains("<svg"),
                "{path}"
            );
        }
        let listed = AppAssets.list("").unwrap();
        assert!(listed.windows(2).all(|p| p[0] < p[1]));
    }
}
