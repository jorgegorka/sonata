pub const RESERVED: &[&str] = &["user"];
const MAX_LEN: usize = 32;

pub fn is_valid_handle(h: &str) -> bool {
    let b = h.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_LEN
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
        && !RESERVED.contains(&h)
}

pub fn derive_handle(display_name: &str, taken: &[String]) -> String {
    let folded = deunicode::deunicode(display_name).to_lowercase();
    let mut base = String::new();
    for c in folded.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let mut base = base.trim_end_matches('-').to_string();
    if base.is_empty() {
        base = "bot".to_string();
    }
    if !base.starts_with(|c: char| c.is_ascii_lowercase()) {
        base = format!("bot-{base}");
    }
    if RESERVED.contains(&base.as_str()) {
        base = format!("{base}-bot");
    }
    base.truncate(MAX_LEN);
    let base = base.trim_end_matches('-').to_string();

    let mut candidate = base.clone();
    let mut n = 2;
    while taken.contains(&candidate) {
        let suffix = format!("-{n}");
        let mut stem = base.clone();
        stem.truncate(MAX_LEN - suffix.len());
        candidate = format!("{}{suffix}", stem.trim_end_matches('-'));
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Vec<String> {
        vec![]
    }

    #[test]
    fn simple_names() {
        assert_eq!(derive_handle("Researcher", &none()), "researcher");
        assert_eq!(derive_handle("Code Reviewer", &none()), "code-reviewer");
    }

    #[test]
    fn accents_are_folded() {
        assert_eq!(derive_handle("Investigación Ñandú", &none()), "investigacion-nandu");
        assert_eq!(derive_handle("Größe Prüfer", &none()), "grosse-prufer");
    }

    #[test]
    fn symbols_only_and_leading_digits() {
        assert_eq!(derive_handle("  !!! ", &none()), "bot");
        assert_eq!(derive_handle("42 Analyst", &none()), "bot-42-analyst");
    }

    #[test]
    fn user_is_never_produced() {
        assert_eq!(derive_handle("User", &none()), "user-bot");
        assert_ne!(derive_handle("USER", &none()), "user");
    }

    #[test]
    fn duplicates_get_numbered() {
        assert_eq!(derive_handle("Writer", &["writer".into()]), "writer-2");
        assert_eq!(derive_handle("Writer", &["writer".into(), "writer-2".into()]), "writer-3");
    }

    #[test]
    fn long_names_fit_including_suffix() {
        let long = "An extremely long bot name that keeps going and going";
        let first = derive_handle(long, &none());
        assert!(first.len() <= 32 && is_valid_handle(&first), "{first}");
        let second = derive_handle(long, std::slice::from_ref(&first));
        assert!(second.len() <= 32 && second.ends_with("-2") && is_valid_handle(&second), "{second}");
    }

    #[test]
    fn every_derived_handle_is_valid() {
        for name in ["🤖 Robot", "日本語", "a", "--x--", "Ünïcödé", "user", "Ça va?", "x_y"] {
            let h = derive_handle(name, &none());
            assert!(is_valid_handle(&h), "{name:?} -> {h:?}");
        }
    }

    #[test]
    fn validation_rules() {
        assert!(is_valid_handle("a"));
        assert!(is_valid_handle("writer_2-b"));
        assert!(!is_valid_handle(""));
        assert!(!is_valid_handle("Writer"));
        assert!(!is_valid_handle("1writer"));
        assert!(!is_valid_handle("user"));
        assert!(!is_valid_handle(&"a".repeat(33)));
    }
}
