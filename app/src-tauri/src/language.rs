use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

pub use sonata_core::engine::LANGUAGES;

pub async fn apple_languages() -> Vec<String> {
    let mut command = Command::new("/usr/bin/defaults");
    command.args(["read", "-g", "AppleLanguages"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    command.kill_on_drop(true);
    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(5), command.output()).await else {
        return Vec::new();
    };
    parse_apple_languages(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_apple_languages(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let entry = line.trim().trim_end_matches(',');
            if entry.is_empty() || entry == "(" || entry == ")" {
                return None;
            }
            let entry = entry.strip_prefix('"').and_then(|e| e.strip_suffix('"')).unwrap_or(entry);
            (!entry.is_empty()).then(|| entry.to_string())
        })
        .collect()
}

pub fn system_language(preferred: &[String]) -> &'static str {
    preferred
        .iter()
        .filter_map(|tag| {
            let primary = tag.split(['-', '_']).next()?.to_ascii_lowercase();
            LANGUAGES.iter().copied().find(|l| *l == primary)
        })
        .next()
        .unwrap_or("en")
}

pub fn resolve(choice: &str, system: &'static str) -> &'static str {
    LANGUAGES.iter().copied().find(|l| *l == choice).unwrap_or(system)
}

#[cfg(test)]
mod tests {
    use sonata_core::engine::SYSTEM_LANGUAGE;

    use super::*;

    #[test]
    fn apple_languages_are_parsed_quoted_or_not() {
        let out = "(\n    \"es-ES\",\n    \"en-US\",\n    de\n)\n";
        assert_eq!(parse_apple_languages(out), vec!["es-ES", "en-US", "de"]);
        assert!(parse_apple_languages("").is_empty());
    }

    #[test]
    fn the_system_language_is_the_first_shipped_one_else_english() {
        let langs = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(system_language(&langs(&["pt-BR", "fr-CA", "en-US"])), "fr");
        assert_eq!(system_language(&langs(&["pt-BR"])), "en");
        assert_eq!(system_language(&[]), "en");
        assert_eq!(system_language(&langs(&["DE"])), "de");
    }

    #[test]
    fn a_choice_wins_over_the_system_language() {
        assert_eq!(resolve("fr", "es"), "fr");
        assert_eq!(resolve(SYSTEM_LANGUAGE, "es"), "es");
        assert_eq!(resolve("pt", "de"), "de");
    }
}
