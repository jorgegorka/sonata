use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde_json::Value;

pub const FILES: [(&str, &str); 4] = [
    ("en", include_str!("../../locales/en.json")),
    ("es", include_str!("../../locales/es.json")),
    ("fr", include_str!("../../locales/fr.json")),
    ("de", include_str!("../../locales/de.json")),
];

static ACTIVE: RwLock<&'static str> = RwLock::new("en");

fn strings() -> &'static HashMap<&'static str, Value> {
    static STRINGS: OnceLock<HashMap<&'static str, Value>> = OnceLock::new();
    STRINGS.get_or_init(|| {
        FILES
            .iter()
            .map(|(code, text)| {
                (*code, serde_json::from_str(text).unwrap_or_else(|e| panic!("locales/{code}.json is valid JSON: {e}")))
            })
            .collect()
    })
}

#[cfg(test)]
pub fn is_supported(code: &str) -> bool {
    FILES.iter().any(|(c, _)| *c == code)
}

pub fn set_language(code: &str) {
    *ACTIVE.write().unwrap() = FILES.iter().map(|(c, _)| *c).find(|c| *c == code).unwrap_or("en");
}

pub fn language() -> &'static str {
    *ACTIVE.read().unwrap()
}

pub fn t(key: &str, vars: &[(&str, &str)]) -> String {
    t_in(language(), key, vars)
}

pub fn t_in(code: &str, key: &str, vars: &[(&str, &str)]) -> String {
    let mut text = lookup(code, key).or_else(|| lookup("en", key)).unwrap_or(key).to_string();
    for (name, value) in vars {
        text = text.replace(&format!("{{{{{name}}}}}"), value);
    }
    text
}

fn lookup(code: &str, key: &str) -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    if let Some(value) =
        key.split('.').try_fold(strings().get(code)?.get("linux")?, |v, part| v.get(part)).and_then(Value::as_str)
    {
        return Some(value);
    }
    key.split('.').try_fold(strings().get(code)?, |v, part| v.get(part)).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_dotted_keys_in_each_language_and_fills_variables() {
        assert_eq!(t_in("en", "menu.about", &[("app", "Sonata")]), "About Sonata");
        assert_eq!(t_in("es", "menu.quit", &[("app", "Sonata")]), "Salir de Sonata");
        assert_eq!(t_in("fr", "menu.settings", &[]), "Réglages…");
        assert_eq!(t_in("de", "menu.hide", &[("app", "Sonata")]), "Sonata ausblenden");
    }

    #[test]
    fn an_unknown_code_falls_back_to_english() {
        set_language("pt");
        assert_eq!(language(), "en");
        assert_eq!(t("menu.settings", &[]), "Settings…");
        assert_eq!(t_in("xx", "menu.settings", &[]), "Settings…");
        assert_eq!(t("menu.nope", &[]), "menu.nope");
        assert!(is_supported("de") && !is_supported("pt"));
    }

    #[test]
    fn every_error_the_ui_can_get_has_a_text_in_every_language() {
        let core = sonata_core::Refusal::examples().into_iter().map(|refusal| refusal.code());
        let app = [
            "internal",
            "trashFailed",
            "skillTrashFailed",
            "fileGone",
            "opensPrograms",
            "openFailed",
            "browserBadUrl",
            "browserFailed",
        ];
        let codes: Vec<&str> = core.chain(app).collect();
        for (lang, _) in FILES {
            for code in &codes {
                let key = format!("errors.{code}");
                assert!(lookup(lang, &key).is_some(), "app/locales/{lang}.json has no {key}");
            }
        }
    }
}
