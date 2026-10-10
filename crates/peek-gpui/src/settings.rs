//! Editable choices shared by the settings menu and persisted configuration.
use peek_core::Config;

#[derive(Clone, Copy)]
pub enum Setting {
    Theme,
    Zoom,
    Language,
    Target,
    ChineseTarget,
    Style,
    Confidence,
    Timeout,
}
impl Setting {
    pub fn key(self) -> &'static str {
        match self {
            Self::Theme => "settings-theme",
            Self::Zoom => "settings-zoom",
            Self::Language => "settings-language",
            Self::Target => "settings-default-target",
            Self::ChineseTarget => "settings-chinese-target",
            Self::Style => "settings-style",
            Self::Confidence => "settings-decision-threshold",
            Self::Timeout => "settings-decision-wait",
        }
    }
    pub fn choices(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Theme => &[
                ("system", "settings-theme-system"),
                ("light", "settings-theme-light"),
                ("dark", "settings-theme-dark"),
            ],
            Self::Zoom => &[
                ("0.8", "80%"),
                ("0.9", "90%"),
                ("1", "100%"),
                ("1.1", "110%"),
                ("1.25", "125%"),
                ("1.5", "150%"),
            ],
            Self::Language => &[
                ("system", "settings-language-system"),
                ("zh-CN", "language-zh"),
                ("en", "language-en"),
            ],
            Self::Target | Self::ChineseTarget => &[
                ("Chinese", "language-zh"),
                ("English", "language-en"),
                ("Japanese", "language-ja"),
                ("Korean", "language-ko"),
                ("French", "language-fr"),
                ("German", "language-de"),
                ("Spanish", "language-es"),
            ],
            Self::Style => &[
                ("natural", "settings-style-natural"),
                ("literal", "settings-style-literal"),
                ("technical", "settings-style-technical"),
            ],
            Self::Confidence => &[
                ("0.5", "50%"),
                ("0.6", "60%"),
                ("0.65", "65%"),
                ("0.7", "70%"),
                ("0.8", "80%"),
                ("0.9", "90%"),
            ],
            Self::Timeout => &[
                ("4000", "4 s"),
                ("8000", "8 s"),
                ("15000", "15 s"),
                ("30000", "30 s"),
            ],
        }
    }
    pub fn value(self, c: &Config) -> String {
        match self {
            Self::Theme => c.theme.clone(),
            Self::Zoom => c.zoom.to_string(),
            Self::Language => c.ui_language.clone(),
            Self::Target => c.target_language.clone(),
            Self::ChineseTarget => c.chinese_target.clone(),
            Self::Style => c.translation_style.clone(),
            Self::Confidence => c.decision.min_confidence.to_string(),
            Self::Timeout => c.decision.timeout_ms.to_string(),
        }
    }
    pub fn assign(self, c: &mut Config, value: &str) {
        match self {
            Self::Theme => c.theme = value.into(),
            Self::Zoom => c.zoom = value.parse().unwrap_or(1.),
            Self::Language => c.ui_language = value.into(),
            Self::Target => c.target_language = value.into(),
            Self::ChineseTarget => c.chinese_target = value.into(),
            Self::Style => c.translation_style = value.into(),
            Self::Confidence => c.decision.min_confidence = value.parse().unwrap_or(0.6),
            Self::Timeout => c.decision.timeout_ms = value.parse().unwrap_or(8000),
        }
    }
}
