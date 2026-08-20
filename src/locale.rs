//! Resolving a client-advertised language code to a locale we ship.
//!
//! The HTTP layer feeds it the X-App-Lang header the frontend sends. The
//! `rust_i18n::i18n!` invocation itself has to stay at the crate root, so only
//! the resolution rules live here.

/// Locale used when a request advertises no language, or one we don't ship.
/// Read from the i18n! macro's constant rather than hardcoded, so the two
/// cannot disagree.
pub fn fallback() -> &'static str {
    crate::_RUST_I18N_FALLBACK_LOCALE
        .and_then(|locales| locales.first())
        .copied()
        .unwrap_or("en")
}

/// Resolve a client-advertised language code to a locale we ship, falling back
/// to the built-in default.
pub fn or_default(code: Option<&str>) -> String {
    code.and_then(resolve)
        .unwrap_or_else(|| fallback().to_string())
}

fn resolve(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_lowercase();
    if code.is_empty() {
        return None;
    }
    let available = rust_i18n::available_locales!();
    if available.iter().any(|a| a.as_ref() == code) {
        return Some(code);
    }
    let base = code.split('-').next()?;
    available
        .iter()
        .any(|a| a.as_ref() == base)
        .then(|| base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_i18n::t;

    #[test]
    fn resolves_a_language_code_to_a_shipped_locale() {
        assert_eq!(resolve("ja"), Some("ja".to_string()));
        assert_eq!(resolve("ja-JP"), Some("ja".to_string()));
        assert_eq!(resolve("en-US"), Some("en".to_string()));
        assert_eq!(resolve("xx-YY"), None);
        assert_eq!(resolve("   "), None);
        assert_eq!(or_default(Some("xx")), fallback());
        assert_eq!(or_default(None), "en");
    }

    #[test]
    fn embedded_catalogs_load_and_substitute() {
        assert_eq!(t!("error_user_not_found", locale = "en"), "User not found.");
        assert_eq!(
            t!("error_user_not_found", locale = "ja"),
            "ユーザーが見つかりません。"
        );
        assert_eq!(
            t!(
                "msg_code_sent",
                locale = "en",
                destination = "a@example.com"
            ),
            "A confirmation code was sent to a@example.com."
        );
    }
}
