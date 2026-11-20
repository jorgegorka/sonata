use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::Watcher;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const EVENT: &str = "omarchy-theme";

const DEBOUNCE: Duration = Duration::from_millis(100);

/// Colors the window can paint. Missing theme keys are filled from the usual
/// fallbacks (`color0`, `color4`, and so on) so the frontend gets one complete set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Palette {
    pub mode: String,
    pub background: String,
    pub panel: String,
    pub text: String,
    pub muted: String,
    pub border: String,
    pub accent: String,
    pub mine: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub orange: String,
    pub console_bg: String,
    pub console_text: String,
}

/// Omarchy writes the live theme under `~/.local/state` even when `XDG_STATE_HOME` points elsewhere.
fn current_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".local/state/omarchy/current"))
}

pub fn read_current() -> Option<Palette> {
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    #[cfg(target_os = "linux")]
    {
        read_palette(&current_dir()?.join("theme").join("colors.toml"))
    }
}

pub fn read_palette(path: &Path) -> Option<Palette> {
    let text = std::fs::read_to_string(path).ok()?;
    parse(&text)
}

pub fn parse(text: &str) -> Option<Palette> {
    let mut colors = HashMap::new();
    let mut mode = None;
    for line in text.lines() {
        let Some((key, value)) = entry(line) else { continue };
        if key == "mode" {
            if value == "light" || value == "dark" {
                mode = Some(value.to_string());
            }
            continue;
        }
        if is_hex(value) {
            colors.insert(key.to_string(), value.to_ascii_lowercase());
        }
    }
    resolve(&colors, mode.as_deref())
}

/// A theme switch deletes `theme/` and moves a new directory in, then rewrites `theme.generation`
/// in place. Those two names are the signal. `theme.name` and the background link change too and are not.
pub fn theme_changed(path: &Path) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("theme" | "theme.generation") => true,
        _ => false,
    }
}

fn entry(line: &str) -> Option<(&str, &str)> {
    let line = line.trim().trim_end_matches('\r');
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let rest = rest.trim();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = rest[1..].split_once(quote)?.0;
    Some((key, value))
}

fn is_hex(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next() == Some('#') && chars.clone().count() == 6 && chars.all(|c| c.is_ascii_hexdigit())
}

fn pick<'a>(colors: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| colors.get(*key).map(String::as_str))
}

fn resolve(colors: &HashMap<String, String>, mode: Option<&str>) -> Option<Palette> {
    let background = pick(colors, &["background", "color0"])?;
    let text = pick(colors, &["foreground", "color7"])?;
    let lighter = colors.get("lighter_background").map(String::as_str);
    let selection = colors.get("selection").map(String::as_str);
    let panel = match lighter {
        Some(color) if color != background => color,
        _ => selection.unwrap_or(background),
    };
    let mine = match selection {
        Some(color) if color != background => color,
        _ => panel,
    };
    let mode = match mode {
        Some("light") => "light",
        Some("dark") => "dark",
        _ if luminance(background) < 0.4 => "dark",
        _ => "light",
    };
    let yellow = pick(colors, &["yellow", "color3"]).unwrap_or("#c96b05");
    Some(Palette {
        mode: mode.to_string(),
        background: background.to_string(),
        panel: panel.to_string(),
        text: text.to_string(),
        muted: pick(colors, &["dark_foreground", "light_foreground"]).unwrap_or(text).to_string(),
        border: pick(colors, &["muted", "color8"]).unwrap_or(text).to_string(),
        accent: pick(colors, &["accent", "color4", "blue"]).unwrap_or(text).to_string(),
        mine: mine.to_string(),
        red: pick(colors, &["red", "color1"]).unwrap_or("#e11d48").to_string(),
        green: pick(colors, &["green", "color2"]).unwrap_or("#059669").to_string(),
        yellow: yellow.to_string(),
        orange: pick(colors, &["orange"]).unwrap_or(yellow).to_string(),
        console_bg: pick(colors, &["darker_background", "dark_background"]).unwrap_or(background).to_string(),
        console_text: pick(colors, &["bright_foreground"]).unwrap_or(text).to_string(),
    })
}

fn channel(byte: u8) -> f64 {
    let c = f64::from(byte) / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(hex: &str) -> f64 {
    let byte = |start: usize| u8::from_str_radix(&hex[start..start + 2], 16).unwrap_or(0);
    0.2126 * channel(byte(1)) + 0.7152 * channel(byte(3)) + 0.0722 * channel(byte(5))
}

/// Keeps the directory watch alive. Dropping it asks the thread to stop.
pub struct Watch {
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Watch {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop();
        if let Ok(mut slot) = self.thread.lock() {
            if let Some(thread) = slot.take() {
                let _ = thread.join();
            }
        }
    }
}

pub fn watch(app: AppHandle) -> Option<Watch> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        None
    }
    #[cfg(target_os = "linux")]
    {
        watch_linux(app)
    }
}

#[cfg(target_os = "linux")]
fn watch_linux(app: AppHandle) -> Option<Watch> {
    let dir = current_dir()?;
    if !dir.is_dir() {
        return None;
    }
    // Watch the parent, not colors.toml. The swap replaces that file's directory, which would
    // leave a watch on the old inode. theme.generation stays put and is rewritten after the swap.
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx).ok()?;
    watcher.watch(&dir, notify::RecursiveMode::NonRecursive).ok()?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("omarchy-theme".into())
        .spawn(move || {
            let _watcher = watcher;
            let mut last: Option<Option<Palette>> = None;
            let mut pending = false;
            let mut deadline = Instant::now();
            loop {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let wait = if pending {
                    deadline.saturating_duration_since(Instant::now())
                } else {
                    Duration::from_millis(250)
                };
                match rx.recv_timeout(wait) {
                    Ok(Ok(event)) => {
                        if event.paths.iter().any(|path| theme_changed(path)) {
                            pending = true;
                            deadline = Instant::now() + DEBOUNCE;
                        }
                    }
                    Ok(Err(error)) => tracing::warn!(%error, "omarchy theme watch failed"),
                    Err(RecvTimeoutError::Timeout) => {
                        if flag.load(Ordering::Relaxed) {
                            break;
                        }
                        if pending && Instant::now() >= deadline {
                            pending = false;
                            let palette = read_palette(&dir.join("theme").join("colors.toml"));
                            if last.as_ref() != Some(&palette) {
                                last = Some(palette.clone());
                                if let Err(error) = app.emit(EVENT, &palette) {
                                    tracing::warn!(%error, "could not tell the window about the theme");
                                }
                            }
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .ok()?;
    Some(Watch { stop, thread: Mutex::new(Some(thread)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO_NIGHT: &str = r##"
mode = "dark"
accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"
background = "#1a1b26"
dark_background = "#13141c"
darker_background = "#0e0e14"
lighter_background = "#24283b"
foreground = "#a9b1d6"
dark_foreground = "#565f89"
light_foreground = "#b4bee6"
bright_foreground = "#c0caf5"
red = "#f7768e"
yellow = "#e0af68"
orange = "#eb927b"
green = "#9ece6a"
"##;

    const LATTE: &str = r##"
mode = "light"
accent = "#1e66f5"
selection = "#ccd0da"
muted = "#acb0be"
background = "#eff1f5"
dark_background = "#e3e4e8"
darker_background = "#d7d8dc"
lighter_background = "#dce0e8"
foreground = "#4c4f69"
dark_foreground = "#9ca0b0"
light_foreground = "#5c5f77"
bright_foreground = "#4c4f69"
red = "#d20f39"
yellow = "#df8e1d"
orange = "#d84e2b"
green = "#40a02b"
"##;

    #[test]
    fn tokyo_night_resolves_its_surfaces() {
        let palette = parse(TOKYO_NIGHT).unwrap();
        assert_eq!(palette.mode, "dark");
        assert_eq!(palette.background, "#1a1b26");
        assert_eq!(palette.panel, "#24283b");
        assert_eq!(palette.text, "#a9b1d6");
        assert_eq!(palette.muted, "#565f89");
        assert_eq!(palette.border, "#414868");
        assert_eq!(palette.accent, "#7aa2f7");
        assert_eq!(palette.mine, "#292e42");
        assert_eq!(palette.console_bg, "#0e0e14");
        assert_eq!(palette.console_text, "#c0caf5");
        assert_eq!(palette.orange, "#eb927b");
    }

    #[test]
    fn latte_is_a_light_theme() {
        let palette = parse(LATTE).unwrap();
        assert_eq!(palette.mode, "light");
        assert_eq!(palette.background, "#eff1f5");
        assert_eq!(palette.panel, "#dce0e8");
        assert_eq!(palette.accent, "#1e66f5");
        assert_eq!(palette.muted, "#9ca0b0");
        assert_eq!(palette.border, "#acb0be");
        assert_eq!(palette.console_bg, "#d7d8dc");
    }

    #[test]
    fn numbered_colors_fill_gaps_and_a_blank_file_does_not() {
        let palette = parse("color0 = \"#111111\"\ncolor7 = '#EEEEEE'\ncolor4 = \"#0000FF\"\n").unwrap();
        assert_eq!(palette.background, "#111111");
        assert_eq!(palette.text, "#eeeeee");
        assert_eq!(palette.accent, "#0000ff");
        assert_eq!(palette.mode, "dark");
        assert!(parse("").is_none());
        assert!(parse("mode = \"dark\"\n# just a comment\n").is_none());
    }

    #[test]
    fn mode_follows_the_background_when_the_file_omits_it() {
        assert_eq!(parse("background = \"#ffffff\"\nforeground = \"#111111\"\n").unwrap().mode, "light");
        assert_eq!(parse("background = \"#111111\"\nforeground = \"#eeeeee\"\n").unwrap().mode, "dark");
    }

    #[test]
    fn a_panel_that_matches_the_background_uses_the_selection() {
        let same = parse(
            "background = \"#111111\"\nlighter_background = \"#111111\"\nselection = \"#222222\"\nforeground = \"#eeeeee\"\n",
        )
        .unwrap();
        assert_eq!(same.panel, "#222222");
        assert_eq!(same.mine, "#222222");
        let selected = parse(
            "background = \"#111111\"\nlighter_background = \"#333333\"\nselection = \"#111111\"\nforeground = \"#eeeeee\"\n",
        )
        .unwrap();
        assert_eq!(selected.panel, "#333333");
        assert_eq!(selected.mine, "#333333");
    }

    #[test]
    fn only_the_theme_directory_and_its_generation_count_as_a_change() {
        let current = Path::new("/home/user/.local/state/omarchy/current");
        assert!(theme_changed(&current.join("theme.generation")));
        assert!(theme_changed(&current.join("theme")));
        assert!(!theme_changed(&current.join("background")));
        assert!(!theme_changed(&current.join("theme.name")));
        assert!(!theme_changed(&current.join("theme").join("colors.toml")));
    }

    #[test]
    fn a_missing_file_is_no_palette() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_palette(&tmp.path().join("colors.toml")).is_none());
    }
}
