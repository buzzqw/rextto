//! Active-language helper for non-UI messages (notifications and the
//! human-readable log summaries). The UI itself is translated in the frontend;
//! this keeps backend messages aligned with the chosen interface language.

use std::sync::{LazyLock, RwLock};

static LANGUAGE: LazyLock<RwLock<String>> = LazyLock::new(|| RwLock::new("it".to_string()));

/// Sets the active language from a public code (`it`, `en`, `ita`, `eng`, ...).
pub fn set_language(lang: &str) {
    let normalized = if lang.trim().to_ascii_lowercase().starts_with("en") {
        "en"
    } else {
        "it"
    };
    *LANGUAGE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = normalized.to_string();
}

pub fn language() -> String {
    LANGUAGE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn is_english() -> bool {
    language() == "en"
}

/// Picks the English variant when the interface language is English, otherwise
/// the Italian one.
pub fn pick<'a>(italian: &'a str, english: &'a str) -> &'a str {
    if is_english() {
        english
    } else {
        italian
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_variant_by_language() {
        set_language("it");
        assert_eq!(pick("ciao", "hello"), "ciao");
        set_language("en");
        assert_eq!(pick("ciao", "hello"), "hello");
        // Restore the default so other tests are unaffected.
        set_language("it");
    }
}
