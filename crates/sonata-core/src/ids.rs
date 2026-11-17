use std::sync::{Mutex, OnceLock};

use ulid::Generator;

static GENERATOR: OnceLock<Mutex<Generator>> = OnceLock::new();

pub fn new_id() -> String {
    let generator = GENERATOR.get_or_init(|| Mutex::new(Generator::new()));
    generator.lock().unwrap().generate().expect("ULID random part overflowed").to_string()
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_ulids_and_strictly_increasing() {
        let ids: Vec<String> = (0..1000).map(|_| new_id()).collect();
        assert!(ids.iter().all(|id| id.len() == 26));
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn now_is_rfc3339_utc() {
        let t = now();
        assert!(t.ends_with('Z'), "{t}");
        assert!(chrono::DateTime::parse_from_rfc3339(&t).is_ok());
    }
}
