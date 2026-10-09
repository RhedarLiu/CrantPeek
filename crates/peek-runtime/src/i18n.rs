//! Fluent resources are the sole source of UI copy. Stable IDs never use visible text.
use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
#[cfg(test)]
use std::collections::BTreeSet;
use std::sync::OnceLock;

thread_local! { static ACTIVE: std::cell::RefCell<I18n> = std::cell::RefCell::new(I18n::new("system")); }
pub fn set_language(language: &str) {
    ACTIVE.with(|active| *active.borrow_mut() = I18n::new(language));
}
pub fn tr(id: &str) -> String {
    ACTIVE.with(|active| active.borrow().text(id))
}
pub fn format(id: &str, parameters: &[(&str, &str)]) -> String {
    ACTIVE.with(|active| active.borrow().format(id, parameters))
}

pub fn protocol(value: peek_core::Protocol) -> String {
    tr(match value {
        peek_core::Protocol::ChatCompletions => "protocol-chat",
        peek_core::Protocol::Responses => "protocol-responses",
        peek_core::Protocol::Anthropic => "protocol-anthropic",
    })
}
pub fn diagnostic(value: &str) -> String {
    if value.starts_with("error-") {
        tr(value)
    } else {
        value.to_owned()
    }
}
impl I18n {
    pub fn network_error(&self, error: &peek_network::Error) -> String {
        match error {
            peek_network::Error::Network(detail) => {
                self.format("error-network", &[("detail", &detail.to_string())])
            }
            peek_network::Error::Http(code) => {
                self.format("error-http", &[("code", &code.to_string())])
            }
            _ => self.text(error.message_key()),
        }
    }
}

const EN: &str = include_str!("../locales/en.ftl");
const ZH: &str = include_str!("../locales/zh-CN.ftl");

pub struct I18n {
    primary: FluentBundle<FluentResource>,
    fallback: FluentBundle<FluentResource>,
}
fn bundle(locale: &str, source: &str) -> FluentBundle<FluentResource> {
    let mut bundle =
        FluentBundle::new_concurrent(vec![locale.parse().expect("valid built-in locale")]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(FluentResource::try_new(source.to_owned()).expect("valid locale resource"))
        .expect("unique message IDs");
    bundle
}
impl I18n {
    pub fn new(language: &str) -> Self {
        static SYSTEM: OnceLock<String> = OnceLock::new();
        let language = if language == "system" {
            SYSTEM
                .get_or_init(|| {
                    // `PEEK_LANGUAGE` forces a language for isolated previews,
                    // which otherwise follow the system locale and so cannot
                    // show the other catalog on one machine.
                    std::env::var("PEEK_LANGUAGE")
                        .ok()
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| sys_locale::get_locale().unwrap_or_else(|| "en".into()))
                })
                .as_str()
        } else {
            language
        };
        let chinese = language.to_ascii_lowercase().starts_with("zh");
        Self {
            primary: bundle(
                if chinese { "zh-CN" } else { "en" },
                if chinese { ZH } else { EN },
            ),
            fallback: bundle("en", EN),
        }
    }
    pub fn text(&self, id: &str) -> String {
        self.format(id, &[])
    }
    pub fn format(&self, id: &str, parameters: &[(&str, &str)]) -> String {
        let mut args = FluentArgs::new();
        for (name, value) in parameters {
            args.set(*name, *value);
        }
        for bundle in [&self.primary, &self.fallback] {
            if let Some(pattern) = bundle.get_message(id).and_then(|m| m.value()) {
                let mut errors = Vec::new();
                let result = bundle.format_pattern(pattern, Some(&args), &mut errors);
                if errors.is_empty() {
                    return result.into_owned();
                }
            }
        }
        // Visible diagnostic rather than silently producing an untranslated blank.
        format!("[{id}]")
    }
    pub fn task(&self, task: peek_core::Task) -> String {
        self.text(match task {
            peek_core::Task::Translate => "task-translate",
            peek_core::Task::Define => "task-define",
            peek_core::Task::ExplainCode => "task-explain-code",
            peek_core::Task::ExplainError => "task-explain-error",
            peek_core::Task::Explain => "task-explain",
        })
    }
}
#[cfg(test)]
fn keys(source: &str) -> BTreeSet<&str> {
    source
        .lines()
        .filter(|l| !l.starts_with([' ', '#']) && l.contains(" = "))
        .filter_map(|l| l.split_once(" = ").map(|(k, _)| k))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Every quoted literal in `source`, with the byte offset of its opening
    /// quote so callers can tell ids from resource references.
    fn quoted_literals(source: &str) -> Vec<(&str, usize)> {
        let mut found = Vec::new();
        let mut search = 0;
        while let Some(offset) = source[search..].find('"') {
            let start = search + offset;
            let Some(close) = source[start + 1..].find('"') else {
                break;
            };
            let end = start + 1 + close;
            found.push((&source[start + 1..end], start));
            search = end + 1;
        }
        found
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn request_locale_survives_async_scheduling() {
        let locale = I18n::new("zh-CN");
        let result = tokio::spawn(async move {
            tokio::task::yield_now().await;
            locale.text("query-clear")
        })
        .await
        .unwrap();
        assert_eq!(result, I18n::new("zh-CN").text("query-clear"));
    }
    #[test]
    fn catalogs_have_identical_keys_and_parse() {
        assert_eq!(keys(EN), keys(ZH));
        for language in ["en", "zh-CN"] {
            let i18n = I18n::new(language);
            for id in keys(EN) {
                assert!(i18n.primary.get_message(id).is_some(), "{language}: {id}");
            }
        }
    }
    #[test]
    fn all_app_resource_references_exist() {
        let available = keys(EN);
        let sources = [
            // This module lives in `peek-runtime`; the view layer it audits is
            // the shell crate.
            include_str!("../../peek-gpui/src/main.rs"),
            include_str!("../../peek-gpui/src/snip.rs"),
            include_str!("permissions.rs"),
            include_str!("store.rs"),
            include_str!("../../peek-core/src/lib.rs"),
            include_str!("../../peek-network/src/lib.rs"),
            include_str!("../../peek-dict/src/lib.rs"),
        ];
        for source in sources {
            for (literal, position) in quoted_literals(source) {
                // A literal handed to a constructor is a stable widget id, not
                // a resource reference: Button::new("look-up"), .id("answer").
                // Without this the audit flags every id shaped like a key.
                let before = &source[position.saturating_sub(12)..position];
                if before.ends_with("new(") || before.ends_with(".id(") {
                    continue;
                }
                if [
                    "query-",
                    "header-",
                    "settings-",
                    "welcome-",
                    "status-",
                    "error-",
                    "route-",
                    "snip-",
                    "tray-",
                    "dict-",
                    "form-",
                    "language-",
                ]
                .iter()
                .any(|prefix| literal.starts_with(prefix))
                    && literal.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                {
                    // Stable widget IDs are deliberately separate from translated labels.
                    if [
                        "settings-answer",
                        "settings-translation",
                        "settings-general",
                        "settings-back",
                        "snip-area",
                    ]
                    .contains(&literal)
                    {
                        continue;
                    }
                    assert!(available.contains(literal), "Missing resource: {literal}");
                }
            }
        }
    }
    #[test]
    fn every_message_formats_with_identical_parameters() {
        fn variables(value: &str) -> BTreeSet<String> {
            value
                .split('$')
                .skip(1)
                .map(|v| {
                    v.chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect()
                })
                .collect()
        }
        let messages = |source: &str| -> std::collections::BTreeMap<String, String> {
            source
                .lines()
                .filter_map(|line| {
                    line.split_once(" = ")
                        .map(|(key, value)| (key.to_owned(), value.to_owned()))
                })
                .collect()
        };
        let english = messages(EN);
        let chinese = messages(ZH);
        for (id, value) in &english {
            let vars = variables(value);
            assert_eq!(vars, variables(&chinese[id]), "Parameter mismatch: {id}");
            let parameters: Vec<_> = vars.iter().map(|v| (v.as_str(), "test")).collect();
            for language in ["en", "zh-CN"] {
                assert!(
                    !I18n::new(language).format(id, &parameters).starts_with('['),
                    "{language}: {id}"
                );
            }
        }
    }
    #[test]
    fn parameters_and_fallback_work() {
        assert_eq!(
            I18n::new("en").format("error-http", &[("code", "429")]),
            "Service returned HTTP 429"
        );
        assert_eq!(I18n::new("zh-CN").text("query-clear"), "清空");
        assert_eq!(I18n::new("fr").text("query-clear"), "Clear");
    }
}
