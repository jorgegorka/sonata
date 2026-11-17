use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;

use crate::model::Provider;
use crate::provider::{adapter_for, LoginCheck};

const MARK: &str = "__SONATA_PATH__";
const LANG_MARK: &str = "__SONATA_LANG__";

const CHECK_LIMIT: Duration = Duration::from_secs(15);

pub fn user_shell() -> PathBuf {
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|shell| shell.is_absolute() && shell.is_file())
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoginShellEnv {
    pub path: String,
    pub lang: Option<String>,
}

pub async fn login_shell_path(shell: &Path, limit: Duration) -> Option<String> {
    login_shell_env(shell, limit).await.map(|env| env.path)
}

pub async fn login_shell_env(shell: &Path, limit: Duration) -> Option<LoginShellEnv> {
    let script = format!("printf '\\n{MARK}%s{MARK}\\n{LANG_MARK}%s{LANG_MARK}\\n' \"$PATH\" \"$LANG\"");
    let mut command = Command::new(shell);
    command
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(limit, command.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let path = between(&text, MARK).filter(|path| !path.is_empty())?;
    Some(LoginShellEnv { path, lang: between(&text, LANG_MARK).filter(|lang| !lang.is_empty()) })
}

fn between(text: &str, mark: &str) -> Option<String> {
    let start = text.find(mark)? + mark.len();
    let end = start + text[start..].find(mark)?;
    Some(text[start..end].trim().to_string())
}

pub async fn apple_locale() -> Option<String> {
    let mut command = Command::new("/usr/bin/defaults");
    command.args(["read", "-g", "AppleLocale"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    command.kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

pub fn utf8_lang(shell_lang: Option<&str>, apple_locale: Option<&str>) -> String {
    utf8_lang_in(Path::new("/usr/share/locale"), shell_lang, apple_locale)
}

fn utf8_lang_in(locale_dir: &Path, shell_lang: Option<&str>, apple_locale: Option<&str>) -> String {
    if let Some(lang) = shell_lang.map(str::trim).filter(|lang| lang.to_ascii_uppercase().contains("UTF-8")) {
        return lang.to_string();
    }
    let region = apple_locale.and_then(|l| l.split('@').next()).map(str::trim).filter(|l| !l.is_empty());
    match region {
        Some(region) if locale_dir.join(region).is_dir() => format!("{region}.UTF-8"),
        _ => "en_US.UTF-8".to_string(),
    }
}

fn usual_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/bin"),
        home.join(".claude/local"),
        home.join(".grok/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

pub fn search_path(login: Option<&str>, home: &Path, inherited: Option<&str>) -> String {
    let usual = usual_dirs(home).into_iter().map(|dir| dir.to_string_lossy().into_owned());
    let all = login
        .unwrap_or("")
        .split(':')
        .map(str::to_string)
        .chain(usual)
        .chain(inherited.unwrap_or("").split(':').map(str::to_string));
    let mut dirs: Vec<String> = Vec::new();
    for dir in all {
        if Path::new(&dir).is_absolute() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.join(":")
}

pub fn find_executable(name: &str, search_path: &str) -> Option<PathBuf> {
    search_path
        .split(':')
        .filter(|dir| Path::new(dir).is_absolute())
        .map(|dir| Path::new(dir).join(name))
        .find(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderStatus {
    pub provider: Provider,
    pub path: Option<String>,
    pub version: Option<String>,
    pub logged_in: Option<bool>,
}

async fn run(exe: &Path, args: &[&str], search_path: &str) -> Option<(bool, String)> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .env("PATH", search_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(CHECK_LIMIT, command.output()).await.ok()?.ok()?;
    Some((output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned()))
}

fn parse_logged_in(json: &str) -> Option<bool> {
    serde_json::from_str::<serde_json::Value>(json).ok()?.get("loggedIn")?.as_bool()
}

fn parse_nonempty_list(json: &str) -> Option<bool> {
    Some(!serde_json::from_str::<serde_json::Value>(json).ok()?.as_array()?.is_empty())
}

pub async fn detect(provider: Provider, search_path: &str) -> ProviderStatus {
    let mut status = ProviderStatus { provider, path: None, version: None, logged_in: None };
    let Some(adapter) = adapter_for(provider) else { return status };
    let Some(exe) = find_executable(adapter.executable(), search_path) else { return status };
    status.path = Some(exe.to_string_lossy().into_owned());
    status.version = match run(&exe, &["--version"], search_path).await {
        Some((true, out)) => out.lines().map(str::trim).find(|line| !line.is_empty()).map(str::to_string),
        _ => None,
    };
    status.logged_in = match adapter.login_check() {
        Some(LoginCheck::Json(args)) => run(&exe, args, search_path).await.and_then(|(_, out)| parse_logged_in(&out)),
        Some(LoginCheck::ExitStatus(args)) => run(&exe, args, search_path).await.map(|(ok, _)| ok),
        Some(LoginCheck::JsonList(args)) => {
            run(&exe, args, search_path).await.and_then(|(_, out)| parse_nonempty_list(&out))
        }
        None => None,
    };
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn codex_reports_its_login_by_exit_status() {
        let tmp = tempfile::tempdir().unwrap();
        let search = tmp.path().to_string_lossy().into_owned();
        script(
            tmp.path(),
            "codex",
            "case \"$1\" in\n  --version) echo 'codex-cli 0.163.0' ;;\n  login) echo 'Not logged in'; exit 1 ;;\nesac",
        );
        let status = detect(Provider::Codex, &search).await;
        assert_eq!((status.version.as_deref(), status.logged_in), (Some("codex-cli 0.163.0"), Some(false)));
        script(tmp.path(), "codex", "case \"$1\" in\n  --version) echo 'codex-cli 0.163.0' ;;\n  login) echo 'Logged in using ChatGPT' ;;\nesac");
        assert_eq!(detect(Provider::Codex, &search).await.logged_in, Some(true));
    }

    #[tokio::test]
    async fn opencode_reports_its_login_by_a_json_list() {
        let tmp = tempfile::tempdir().unwrap();
        let search = tmp.path().to_string_lossy().into_owned();
        let opencode = |list: &str| {
            format!(
                "[ \"$1\" = --version ] && echo 'opencode v2.2.7' && exit 0\n\
                 [ \"$*\" = 'auth list --standalone --format json' ] && echo '{list}'"
            )
        };
        script(tmp.path(), "opencode", &opencode("[]"));
        let status = detect(Provider::Opencode, &search).await;
        assert_eq!((status.version.as_deref(), status.logged_in), (Some("opencode v2.2.7"), Some(false)));
        script(tmp.path(), "opencode", &opencode(r#"[{"id":"opencode-go"}]"#));
        assert_eq!(detect(Provider::Opencode, &search).await.logged_in, Some(true));
        script(tmp.path(), "opencode", &opencode("not json"));
        assert_eq!(detect(Provider::Opencode, &search).await.logged_in, None);
    }

    #[tokio::test]
    async fn the_login_shell_path_is_read_between_the_marks() {
        let tmp = tempfile::tempdir().unwrap();
        let shell = script(
            tmp.path(),
            "fake-shell",
            "echo 'Last login: today'\nPATH=/fake/bin:/usr/bin:/bin\nexport PATH\nshift 3\nexec /bin/sh -c \"$1\"",
        );
        assert_eq!(login_shell_path(&shell, Duration::from_secs(10)).await.as_deref(), Some("/fake/bin:/usr/bin:/bin"));
    }

    #[tokio::test]
    async fn the_login_shells_lang_is_read_with_its_path() {
        let tmp = tempfile::tempdir().unwrap();
        let shell = script(
            tmp.path(),
            "fake-shell",
            "PATH=/fake/bin\nLANG=es_ES.UTF-8\nexport PATH LANG\nshift 3\nexec /bin/sh -c \"$1\"",
        );
        let env = login_shell_env(&shell, Duration::from_secs(10)).await.unwrap();
        assert_eq!(env, LoginShellEnv { path: "/fake/bin".into(), lang: Some("es_ES.UTF-8".into()) });
        let no_lang = script(
            tmp.path(),
            "no-lang-shell",
            "PATH=/fake/bin\nexport PATH\nunset LANG\nshift 3\nexec /bin/sh -c \"$1\"",
        );
        assert_eq!(login_shell_env(&no_lang, Duration::from_secs(10)).await.unwrap().lang, None);
    }

    #[test]
    fn the_pane_lang_is_the_shells_utf8_one_else_the_regions_else_english() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("es_ES")).unwrap();
        let lang = |shell: Option<&str>, apple: Option<&str>| utf8_lang_in(dir.path(), shell, apple);
        assert_eq!(lang(Some("de_DE.utf-8"), Some("es_ES")), "de_DE.utf-8", "the shell's UTF-8 locale wins");
        assert_eq!(lang(Some("C"), Some("es_ES@rg=eszzzz")), "es_ES.UTF-8", "not UTF-8: the region's, without @…");
        assert_eq!(lang(None, Some("es_ES")), "es_ES.UTF-8");
        assert_eq!(lang(None, Some("xx_YY")), "en_US.UTF-8", "a region without a locale folder");
        assert_eq!(lang(Some("en_US.ISO8859-1"), None), "en_US.UTF-8");
        assert_eq!(utf8_lang(None, Some("es_ES")), "es_ES.UTF-8");
    }

    #[tokio::test]
    async fn a_slow_or_failing_shell_gives_no_path() {
        let tmp = tempfile::tempdir().unwrap();
        let slow = script(tmp.path(), "slow-shell", "sleep 5");
        let started = std::time::Instant::now();
        assert_eq!(login_shell_path(&slow, Duration::from_millis(300)).await, None);
        assert!(started.elapsed() < Duration::from_secs(3), "the limit was not applied");
        let failing = script(tmp.path(), "failing-shell", "exit 1");
        assert_eq!(login_shell_path(&failing, Duration::from_secs(10)).await, None);
    }

    #[test]
    fn the_search_path_puts_the_shell_first_and_drops_repeats_and_relative_entries() {
        let home = Path::new("/Users/u");
        assert_eq!(
            search_path(Some("/opt/homebrew/bin:.::/usr/bin"), home, Some("/usr/bin:/bin")),
            "/opt/homebrew/bin:/usr/bin:/Users/u/.local/bin:/Users/u/.claude/local:/Users/u/.grok/bin:\
             /usr/local/bin:/bin"
        );
        assert!(search_path(None, home, None).starts_with("/Users/u/.local/bin:"), "the usual folders without a shell");
    }

    #[test]
    fn only_executable_files_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b, c) = (tmp.path().join("a"), tmp.path().join("b"), tmp.path().join("c"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(b.join("claude")).unwrap();
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(a.join("claude"), "not executable").unwrap();
        let search = format!("{}:{}", a.display(), b.display());
        assert_eq!(find_executable("claude", &search), None);
        let exe = script(&c, "claude", "true");
        assert_eq!(find_executable("claude", &format!("{search}:{}", c.display())), Some(exe));
    }

    #[tokio::test]
    async fn detect_runs_the_cli_with_the_search_path_so_its_helpers_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let (cli, helpers) = (tmp.path().join("cli"), tmp.path().join("helpers"));
        std::fs::create_dir_all(&cli).unwrap();
        std::fs::create_dir_all(&helpers).unwrap();
        script(&helpers, "node", "echo '2.1.0 (Claude Code)'");
        let claude = script(
            &cli,
            "claude",
            "case \"$1\" in\n  --version) exec node ;;\n  auth) echo '{\"loggedIn\": false}'; exit 1 ;;\nesac\nexit 2",
        );
        let search = format!("{}:{}", cli.display(), helpers.display());
        assert_eq!(
            detect(Provider::Claude, &search).await,
            ProviderStatus {
                provider: Provider::Claude,
                path: Some(claude.to_string_lossy().into_owned()),
                version: Some("2.1.0 (Claude Code)".into()),
                logged_in: Some(false),
            }
        );
    }

    #[tokio::test]
    async fn grok_reports_no_login_state_and_a_missing_cli_reports_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let search = tmp.path().to_string_lossy().into_owned();
        assert_eq!(
            detect(Provider::Grok, &search).await,
            ProviderStatus { provider: Provider::Grok, path: None, version: None, logged_in: None }
        );
        let grok = script(tmp.path(), "grok", "echo 'grok 1.0.46 (2765805b9442) [stable]'");
        let status = detect(Provider::Grok, &search).await;
        assert_eq!(status.path, Some(grok.to_string_lossy().into_owned()));
        assert_eq!(status.version.as_deref(), Some("grok 1.0.46 (2765805b9442) [stable]"));
        assert_eq!(status.logged_in, None);
    }
}
