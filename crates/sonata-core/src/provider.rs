use std::path::PathBuf;

use crate::limits::{claude_limit, codex_limit, grok_limit, opencode_limit, Skip, UsageLimit};
use crate::model::{Bot, Provider};
use crate::runtime::TrustPrompt;

pub const CLAUDE_TRUST_OPTION: &str = "Yes, I trust this folder";

pub const CODEX_TRUST_OPTION: &str = "Trust and continue";

pub const CODEX_SIGN_IN_TEXT: &str = "1. Sign in with ChatGPT";

pub const GROK_TRUST_QUESTION: &str = "Do you trust the contents of this directory?";

fn model_of(bot: &Bot) -> Option<&str> {
    bot.model.as_deref().map(str::trim).filter(|m| !m.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginCheck {
    Json(&'static [&'static str]),
    ExitStatus(&'static [&'static str]),
    JsonList(&'static [&'static str]),
}

pub trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> Provider;
    fn executable(&self) -> &'static str;
    fn login_check(&self) -> Option<LoginCheck> {
        None
    }
    fn herdr_kind(&self) -> &'static str;
    fn launch_args(&self, bot: &Bot, resume: Option<&str>) -> Vec<String>;
    fn new_session_args(&self, _session_id: &str) -> Option<Vec<String>> {
        None
    }
    fn cli_session_id(&self, app_id: &str) -> String {
        app_id.to_string()
    }
    fn launch_env(&self, _bot: &Bot) -> Vec<(String, String)> {
        Vec::new()
    }
    fn interrupt_keys(&self) -> &'static [&'static str] {
        &["esc"]
    }
    fn extra_context_files(&self) -> Vec<(PathBuf, String)>;
    fn skill_dir(&self) -> &'static str;
    fn trust_prompt(&self) -> Option<TrustPrompt>;
    fn rereads_context_on_resume(&self) -> bool;
    fn keeps_its_model_on_resume(&self) -> bool {
        false
    }
    fn background_work(&self, _screen: &str) -> bool {
        false
    }
    fn reads_background_work(&self) -> bool {
        false
    }
    fn wakes_after_background_work(&self) -> bool {
        true
    }
    fn current_step(&self, _screen: &str) -> Option<String> {
        None
    }
    fn reads_step(&self) -> bool {
        false
    }
    fn sign_in_text(&self) -> Option<&'static str> {
        None
    }
    fn session_id_from_sbot(&self) -> bool {
        false
    }
    fn usage_limit(&self, _screen: &str, _skip: &Skip) -> Option<UsageLimit> {
        None
    }
}

pub struct ClaudeAdapter;

impl ProviderAdapter for ClaudeAdapter {
    fn rereads_context_on_resume(&self) -> bool {
        true
    }

    fn provider(&self) -> Provider {
        Provider::Claude
    }

    fn executable(&self) -> &'static str {
        "claude"
    }

    fn login_check(&self) -> Option<LoginCheck> {
        Some(LoginCheck::Json(&["auth", "status", "--json"]))
    }

    fn herdr_kind(&self) -> &'static str {
        "claude"
    }

    fn launch_args(&self, bot: &Bot, resume: Option<&str>) -> Vec<String> {
        let mut args = vec![
            "--dangerously-skip-permissions".to_string(),
            "--settings".to_string(),
            r#"{"skipDangerousModePermissionPrompt":true}"#.to_string(),
            "--disallowedTools".to_string(),
            "AskUserQuestion".to_string(),
        ];
        if let Some(model) = model_of(bot) {
            args.push("--model".into());
            args.push(model.into());
        }
        if let Some(id) = resume {
            args.push("--resume".into());
            args.push(id.into());
        }
        args
    }

    fn new_session_args(&self, session_id: &str) -> Option<Vec<String>> {
        Some(vec!["--session-id".into(), session_id.into()])
    }

    fn extra_context_files(&self) -> Vec<(PathBuf, String)> {
        vec![(PathBuf::from("CLAUDE.md"), "@AGENTS.md\n".to_string())]
    }

    fn skill_dir(&self) -> &'static str {
        ".claude/skills"
    }

    fn trust_prompt(&self) -> Option<TrustPrompt> {
        Some(TrustPrompt::Menu { option: CLAUDE_TRUST_OPTION })
    }

    fn usage_limit(&self, screen: &str, skip: &Skip) -> Option<UsageLimit> {
        claude_limit(screen, skip)
    }

    fn background_work(&self, screen: &str) -> bool {
        claude_background_work(screen)
    }

    fn reads_background_work(&self) -> bool {
        true
    }

    fn current_step(&self, screen: &str) -> Option<String> {
        claude_step(screen)
    }

    fn reads_step(&self) -> bool {
        true
    }
}

fn claude_step(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.lines().collect();
    let rules: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| is_rule(line)).map(|(i, _)| i).collect();
    let top = *rules.get(rules.len().checked_sub(2)?)?;
    for at in (0..top).rev() {
        let line = lines[at];
        if line.starts_with("❯ ") {
            return None;
        }
        let Some(text) = line.strip_prefix("⏺ ").or_else(|| {
            line.strip_prefix("  ").filter(|rest| rest.starts_with(|c: char| !c.is_whitespace() && c != '⎿'))
        }) else {
            continue;
        };
        let text = without_timer(text.trim());
        let tool = lines.get(at + 1).is_some_and(|next| next.starts_with("  ⎿  "))
            || (text.starts_with("Running ") && text.ends_with('…'));
        if tool {
            let step = text.trim_end_matches('…').trim_end();
            return (!step.is_empty()).then(|| step.to_string());
        }
        if line.starts_with("⏺ ") {
            return None;
        }
    }
    None
}

fn without_timer(text: &str) -> &str {
    let is_timer = |timer: &str| {
        timer.split(' ').all(|part| {
            part.len() > 1
                && part.ends_with(['h', 'm', 's'])
                && part[..part.len() - 1].chars().all(|c| c.is_ascii_digit())
        })
    };
    match text.rsplit_once(" · ") {
        Some((head, timer)) if is_timer(timer) => head.trim_end(),
        _ => text,
    }
}

fn claude_background_work(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let rules: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| is_rule(line)).map(|(i, _)| i).collect();
    let Some(&bottom) = rules.last() else { return false };
    let shells = lines[bottom + 1..].iter().any(|line| line.split('·').any(|part| is_shell_count(part.trim())));
    let waiting = rules.len() >= 2
        && lines[..rules[rules.len() - 2]]
            .iter()
            .rev()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| line.contains("Waiting for ") && line.contains(" background agent"));
    shells || waiting
}

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.chars().all(|c| c == '─')
}

fn is_shell_count(part: &str) -> bool {
    match part.split_once(' ') {
        Some((count, "shell" | "shells")) => !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()),
        _ => false,
    }
}

pub struct CodexAdapter;

impl ProviderAdapter for CodexAdapter {
    fn rereads_context_on_resume(&self) -> bool {
        true
    }

    fn provider(&self) -> Provider {
        Provider::Codex
    }

    fn executable(&self) -> &'static str {
        "codex"
    }

    fn login_check(&self) -> Option<LoginCheck> {
        Some(LoginCheck::ExitStatus(&["login", "status"]))
    }

    fn herdr_kind(&self) -> &'static str {
        "codex"
    }

    fn launch_args(&self, bot: &Bot, resume: Option<&str>) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(id) = resume {
            args.extend(["resume".to_string(), id.to_string()]);
        }
        args.extend(
            ["--dangerously-bypass-approvals-and-sandbox", "--no-daemon", "-c", "check_for_update_on_startup=false"]
                .map(String::from),
        );
        if let Some(model) = model_of(bot) {
            args.extend(["-m".to_string(), model.to_string()]);
        }
        args
    }

    fn extra_context_files(&self) -> Vec<(PathBuf, String)> {
        Vec::new()
    }

    fn skill_dir(&self) -> &'static str {
        ".agents/skills"
    }

    fn trust_prompt(&self) -> Option<TrustPrompt> {
        Some(TrustPrompt::Menu { option: CODEX_TRUST_OPTION })
    }

    fn usage_limit(&self, screen: &str, skip: &Skip) -> Option<UsageLimit> {
        codex_limit(screen, skip)
    }

    fn background_work(&self, screen: &str) -> bool {
        screen.lines().any(|line| is_background_terminals(line.trim()))
    }

    fn reads_background_work(&self) -> bool {
        true
    }

    fn wakes_after_background_work(&self) -> bool {
        false
    }

    fn sign_in_text(&self) -> Option<&'static str> {
        Some(CODEX_SIGN_IN_TEXT)
    }

    fn session_id_from_sbot(&self) -> bool {
        true
    }
}

fn is_background_terminals(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && (line[digits..].starts_with(" background terminal running")
            || line[digits..].starts_with(" background terminals running"))
}

pub struct GrokAdapter;

impl ProviderAdapter for GrokAdapter {
    fn rereads_context_on_resume(&self) -> bool {
        false
    }

    fn usage_limit(&self, screen: &str, skip: &Skip) -> Option<UsageLimit> {
        grok_limit(screen, skip)
    }

    fn provider(&self) -> Provider {
        Provider::Grok
    }

    fn executable(&self) -> &'static str {
        "grok"
    }

    fn herdr_kind(&self) -> &'static str {
        "grok"
    }

    fn launch_args(&self, bot: &Bot, resume: Option<&str>) -> Vec<String> {
        let mut args = vec!["--always-approve".to_string()];
        if let Some(model) = model_of(bot) {
            args.extend(["-m".to_string(), model.to_string()]);
        }
        if let Some(id) = resume {
            args.extend(["--resume".to_string(), id.to_string()]);
        }
        args
    }

    fn new_session_args(&self, session_id: &str) -> Option<Vec<String>> {
        Some(vec!["-s".into(), session_id.into()])
    }

    fn extra_context_files(&self) -> Vec<(PathBuf, String)> {
        Vec::new()
    }

    fn skill_dir(&self) -> &'static str {
        ".agents/skills"
    }

    fn trust_prompt(&self) -> Option<TrustPrompt> {
        Some(TrustPrompt::Key { question: GROK_TRUST_QUESTION, key: "y" })
    }

    fn current_step(&self, screen: &str) -> Option<String> {
        grok_step(screen)
    }

    fn reads_step(&self) -> bool {
        true
    }
}

fn grok_step(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.lines().collect();
    let above = match lines.iter().rposition(|line| line.trim_start().starts_with('╭')) {
        Some(top) => &lines[..top],
        None => &lines[..],
    };
    let text = above.iter().rev().find_map(|line| {
        let rest =
            line.trim_start().strip_prefix(|c: char| ('\u{2800}'..='\u{28FF}').contains(&c))?.strip_prefix(' ')?;
        rest.find("… ").filter(|&end| end > 0).map(|end| &rest[..end])
    })?;
    (!matches!(text, "Waiting for response" | "Thinking")).then(|| text.to_string())
}

pub struct OpencodeAdapter;

impl ProviderAdapter for OpencodeAdapter {
    fn rereads_context_on_resume(&self) -> bool {
        true
    }

    fn provider(&self) -> Provider {
        Provider::Opencode
    }

    fn executable(&self) -> &'static str {
        "opencode"
    }

    fn login_check(&self) -> Option<LoginCheck> {
        Some(LoginCheck::JsonList(&["auth", "list", "--standalone", "--format", "json"]))
    }

    fn herdr_kind(&self) -> &'static str {
        "opencode"
    }

    fn launch_args(&self, _bot: &Bot, resume: Option<&str>) -> Vec<String> {
        let mut args = vec!["--standalone".to_string(), "--auto".to_string()];
        if let Some(id) = resume {
            args.extend(["-s".to_string(), id.to_string()]);
        }
        args
    }

    fn new_session_args(&self, session_id: &str) -> Option<Vec<String>> {
        Some(vec!["-s".into(), session_id.into()])
    }

    fn cli_session_id(&self, app_id: &str) -> String {
        format!("ses_{}", app_id.replace('-', "").to_ascii_lowercase())
    }

    fn launch_env(&self, bot: &Bot) -> Vec<(String, String)> {
        let mut config = serde_json::json!({
            "permission": {"question": "deny"},
            "websearch": {"provider": "random"},
        });
        if let Some(model) = model_of(bot) {
            config["model"] = model.into();
        }
        vec![
            ("OPENCODE_CONFIG_CONTENT".to_string(), config.to_string()),
            ("OPENCODE_DISABLE_AUTOUPDATE".to_string(), "1".to_string()),
        ]
    }

    fn keeps_its_model_on_resume(&self) -> bool {
        true
    }

    fn interrupt_keys(&self) -> &'static [&'static str] {
        &["esc", "esc"]
    }

    fn extra_context_files(&self) -> Vec<(PathBuf, String)> {
        Vec::new()
    }

    fn skill_dir(&self) -> &'static str {
        ".agents/skills"
    }

    fn trust_prompt(&self) -> Option<TrustPrompt> {
        None
    }

    fn background_work(&self, screen: &str) -> bool {
        screen.lines().rfind(|line| !line.trim().is_empty()).is_some_and(shows_shells)
    }

    fn usage_limit(&self, screen: &str, skip: &Skip) -> Option<UsageLimit> {
        opencode_limit(screen, skip)
    }

    fn reads_background_work(&self) -> bool {
        true
    }
}

fn shows_shells(status: &str) -> bool {
    status.match_indices("↓ ").any(|(at, mark)| {
        let rest = &status[at + mark.len()..];
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let after = rest[digits..].strip_prefix(" shells").or_else(|| rest[digits..].strip_prefix(" shell"));
        digits > 0 && after.is_some_and(|after| after.is_empty() || after.starts_with([' ', '·']))
    })
}

pub fn adapter_for(provider: Provider) -> Option<&'static dyn ProviderAdapter> {
    match provider {
        Provider::Claude => Some(&ClaudeAdapter),
        Provider::Codex => Some(&CodexAdapter),
        Provider::Grok => Some(&GrokAdapter),
        Provider::Opencode => Some(&OpencodeAdapter),
        Provider::Cursor => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_bot;

    #[test]
    fn adapters_name_their_cli_and_how_it_reports_a_login() {
        assert_eq!((ClaudeAdapter.executable(), GrokAdapter.executable()), ("claude", "grok"));
        assert_eq!(ClaudeAdapter.login_check(), Some(LoginCheck::Json(&["auth", "status", "--json"])));
        assert_eq!(GrokAdapter.login_check(), None);
        assert!(ClaudeAdapter.rereads_context_on_resume() && !GrokAdapter.rereads_context_on_resume());
    }

    const AUTONOMOUS: [&str; 5] = [
        "--dangerously-skip-permissions",
        "--settings",
        r#"{"skipDangerousModePermissionPrompt":true}"#,
        "--disallowedTools",
        "AskUserQuestion",
    ];

    #[test]
    fn claude_runs_autonomously_by_default() {
        assert_eq!(ClaudeAdapter.launch_args(&test_bot("x", None), None), AUTONOMOUS.to_vec());
    }

    #[test]
    fn claude_passes_model_and_resume() {
        let mut expected = AUTONOMOUS.to_vec();
        expected.extend(["--model", "sonnet", "--resume", "abc-123"]);
        assert_eq!(ClaudeAdapter.launch_args(&test_bot("x", Some(" sonnet ")), Some("abc-123")), expected);
        assert_eq!(ClaudeAdapter.launch_args(&test_bot("x", Some("  ")), None), AUTONOMOUS.to_vec());
    }

    #[test]
    fn claude_takes_an_app_chosen_session_id() {
        assert_eq!(
            ClaudeAdapter.new_session_args("6f1c0f9e-2b7a-4c39-9d3e-0f5a1b2c3d4e"),
            Some(vec!["--session-id".to_string(), "6f1c0f9e-2b7a-4c39-9d3e-0f5a1b2c3d4e".to_string()])
        );
    }

    #[test]
    fn claude_imports_agents_md() {
        assert_eq!(
            ClaudeAdapter.extra_context_files(),
            vec![(std::path::PathBuf::from("CLAUDE.md"), "@AGENTS.md\n".to_string())]
        );
        assert_eq!(ClaudeAdapter.herdr_kind(), "claude");
    }

    #[test]
    fn claude_answers_its_trust_menu() {
        use crate::runtime::TrustPrompt;
        assert_eq!(ClaudeAdapter.trust_prompt(), Some(TrustPrompt::Menu { option: "Yes, I trust this folder" }));
    }

    #[test]
    fn adapters_name_their_native_skill_folder() {
        assert_eq!(ClaudeAdapter.skill_dir(), ".claude/skills");
        assert_eq!(GrokAdapter.skill_dir(), ".agents/skills", "grok reads several; one copy avoids duplicates (S5)");
    }

    #[test]
    fn cursor_is_not_in_v1() {
        assert!(adapter_for(Provider::Claude).is_some());
        assert!(adapter_for(Provider::Grok).is_some());
        assert!(adapter_for(Provider::Codex).is_some());
        assert!(adapter_for(Provider::Opencode).is_some());
        assert!(adapter_for(Provider::Cursor).is_none());
    }

    #[test]
    fn grok_runs_autonomously_with_model_resume_and_its_trust_answer() {
        use crate::runtime::TrustPrompt;
        assert_eq!(GrokAdapter.launch_args(&test_bot("x", None), None), vec!["--always-approve"]);
        assert_eq!(
            GrokAdapter.launch_args(&test_bot("x", Some(" grok-4 ")), Some("abc")),
            vec!["--always-approve", "-m", "grok-4", "--resume", "abc"]
        );
        assert_eq!(GrokAdapter.new_session_args("u-1"), Some(vec!["-s".to_string(), "u-1".to_string()]));
        assert!(GrokAdapter.extra_context_files().is_empty());
        assert_eq!(GrokAdapter.herdr_kind(), "grok");
        assert_eq!(
            GrokAdapter.trust_prompt(),
            Some(TrustPrompt::Key { question: "Do you trust the contents of this directory?", key: "y" })
        );
    }

    fn claude_screen(above: &str, footer: &str) -> String {
        let rule = "─".repeat(120);
        format!("{above}\n{rule}\n❯ \n{rule}\n{footer}\n")
    }

    #[test]
    fn claude_shows_a_background_shell_in_its_footer() {
        let one = claude_screen(
            "✻ Cogitated for 23s · done 11:37 AM · 1 shell still running",
            "  ⏵⏵ bypass permissions on · 1 shell · ← 1 agent · ↓ to manage",
        );
        assert!(ClaudeAdapter.background_work(&one));
        let two = claude_screen("", "  ⏵⏵ bypass permissions on · 2 shells · ↓ to manage");
        assert!(ClaudeAdapter.background_work(&two));
    }

    #[test]
    fn claude_names_its_latest_tool_call_as_the_step() {
        let screen = include_str!("../tests/fixtures/screens/claude-working.txt");
        assert_eq!(ClaudeAdapter.current_step(screen).as_deref(), Some("Reading shared memory file"));
        let grouped = claude_screen("⏺ Running 1 shell command…\n✶ Architecting… (2s)", "  ⏵⏵ bypass permissions on");
        assert_eq!(ClaudeAdapter.current_step(&grouped).as_deref(), Some("Running 1 shell command"));
    }

    #[test]
    fn claude_names_the_same_step_while_its_marker_blinks() {
        let on = include_str!("../tests/fixtures/screens/claude-working-marker-on.txt");
        let off = include_str!("../tests/fixtures/screens/claude-working-marker-off.txt");
        assert_eq!(ClaudeAdapter.current_step(on).as_deref(), Some("Spelling it out"));
        assert_eq!(ClaudeAdapter.current_step(off).as_deref(), Some("Spelling it out"));
    }

    #[test]
    fn claude_names_the_running_step_and_not_the_finished_one_above_it() {
        let earlier = "⏺ Read(src/lib.rs)\n  ⎿  Read 120 lines\n\n";
        for marker in ["⏺ ", "  "] {
            let screen = claude_screen(
                &format!("{earlier}{marker}Bash(cargo test) · 1m 5s\n  ⎿  Running…\n✶ Architecting… (2s)"),
                "  ⏵⏵ bypass permissions on",
            );
            assert_eq!(ClaudeAdapter.current_step(&screen).as_deref(), Some("Bash(cargo test)"), "{marker:?}");
            let grouped = claude_screen(
                &format!("{earlier}{marker}Running 2 shell commands… · 12s\n✶ Architecting… (2s)"),
                "  ⏵⏵ bypass permissions on",
            );
            assert_eq!(ClaudeAdapter.current_step(&grouped).as_deref(), Some("Running 2 shell commands"), "{marker:?}");
        }
    }

    #[test]
    fn claude_without_a_tool_call_has_no_step() {
        let screen = claude_screen("⏺ I'll read the tests first.", "  ⏵⏵ bypass permissions on");
        assert_eq!(ClaudeAdapter.current_step(&screen), None);
        assert_eq!(ClaudeAdapter.current_step(""), None);
        let below = claude_screen("⏺ Sure, one moment.", "  ⏺ main\n  ⎿  $ ls");
        assert_eq!(ClaudeAdapter.current_step(&below), None);
        let after = claude_screen("⏺ Read(a.rs)\n  ⎿  Read 3 lines\n\n⏺ The tests\n  pass.", "  ⏵⏵ bypass");
        assert_eq!(ClaudeAdapter.current_step(&after), None);
        let prompt = claude_screen(
            "⏺ Bash(ls)\n  ⎿  a.rs\n\n❯ [Message from user]\n  Fix the build\n\n✶ Thinking… (1s)",
            "  ⏵⏵ bypass",
        );
        assert_eq!(ClaudeAdapter.current_step(&prompt), None);
    }

    #[test]
    fn codex_has_no_step() {
        let screen = include_str!("../tests/fixtures/screens/codex-working.txt");
        assert_eq!(CodexAdapter.current_step(screen), None);
    }

    #[test]
    fn adapters_say_which_screen_readings_they_have() {
        let reads = |a: &dyn ProviderAdapter| (a.reads_step(), a.reads_background_work());
        assert_eq!(reads(&ClaudeAdapter), (true, true));
        assert_eq!(reads(&CodexAdapter), (false, true));
        assert_eq!(reads(&GrokAdapter), (true, false));
        assert_eq!(reads(&OpencodeAdapter), (false, true));
    }

    #[test]
    fn adapters_read_their_own_limit_screens() {
        let claude = include_str!("../tests/fixtures/screens/claude-limit.txt");
        let codex = include_str!("../tests/fixtures/screens/codex-limit.txt");
        let grok = include_str!("../tests/fixtures/screens/grok-limit.txt");
        let claude_working = include_str!("../tests/fixtures/screens/claude-working.txt");
        let codex_working = include_str!("../tests/fixtures/screens/codex-working.txt");
        let grok_working = include_str!("../tests/fixtures/screens/grok-working.txt");
        let none = Skip::default();
        assert!(ClaudeAdapter.usage_limit(claude, &none).is_some());
        assert!(CodexAdapter.usage_limit(codex, &none).is_some());
        assert!(GrokAdapter.usage_limit(grok, &none).is_some());
        assert_eq!(ClaudeAdapter.usage_limit(codex_working, &none), None);
        assert_eq!(CodexAdapter.usage_limit(grok_working, &none), None);
        assert_eq!(GrokAdapter.usage_limit(claude_working, &none), None);
        assert_eq!(ClaudeAdapter.usage_limit(claude_working, &none), None);
        let opencode = include_str!("../tests/fixtures/screens/opencode-limit.txt");
        assert!(OpencodeAdapter.usage_limit(opencode, &none).is_some());
        assert_eq!(OpencodeAdapter.usage_limit(claude_working, &none), None);
        assert_eq!(ClaudeAdapter.usage_limit(opencode, &none), None);
    }

    #[test]
    fn grok_names_its_spinner_line_as_the_step() {
        let screen = include_str!("../tests/fixtures/screens/grok-working.txt");
        assert_eq!(GrokAdapter.current_step(screen).as_deref(), Some("Sleep 25 seconds then echo done"));
    }

    #[test]
    fn grok_states_and_summaries_are_no_step() {
        let input = "  ╭──╮\n  │ ❯ │\n  ╰──╯";
        for state in ["Waiting for response", "Thinking"] {
            let screen = format!("   ⠸ {state}… 3s   4s\n\n{input}");
            assert_eq!(GrokAdapter.current_step(&screen), None, "{state}");
        }
        let summary = format!("     ◈ Exploring MEMORY.md, ran 1 command\n\n{input}");
        assert_eq!(GrokAdapter.current_step(&summary), None);
        let longer = format!("   ⠸ Thinking about it… 3s\n\n{input}");
        assert_eq!(GrokAdapter.current_step(&longer).as_deref(), Some("Thinking about it"));
    }

    #[test]
    fn claude_shows_the_subagents_it_waits_for_above_its_prompt() {
        let screen = claude_screen(
            "  they finish.\n✻ Waiting for 1 background agent to finish",
            "  ⏵⏵ bypass permissions on · ← 1 agent · ↓ to manage\n  ⏺ main\n  ◯ general-purpose  Run sleep 100   4s",
        );
        assert!(ClaudeAdapter.background_work(&screen));
    }

    #[test]
    fn claude_with_nothing_left_in_the_background_shows_none() {
        let done = claude_screen(
            "⏺ done\n✻ Cooked for 2s · done 11:40 AM",
            "  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← 1 agent",
        );
        assert!(!ClaudeAdapter.background_work(&done), "`← 1 agent` stays after the agent finished");
        let told = claude_screen("I started 1 shell · it runs the specs", "  ⏵⏵ bypass permissions on");
        assert!(!ClaudeAdapter.background_work(&told), "text above the prompt box is not the footer");
        assert!(!ClaudeAdapter.background_work(""));
        assert!(!ClaudeAdapter.background_work("no prompt box · 1 shell"));
    }

    #[test]
    fn grok_never_reports_background_work() {
        let screen = claude_screen("", "  ⏵⏵ bypass permissions on · 1 shell");
        assert!(!GrokAdapter.background_work(&screen));
    }

    const CODEX: [&str; 4] =
        ["--dangerously-bypass-approvals-and-sandbox", "--no-daemon", "-c", "check_for_update_on_startup=false"];

    #[test]
    fn codex_runs_autonomously_without_the_shared_daemon() {
        assert_eq!(CodexAdapter.launch_args(&test_bot("x", None), None), CODEX.to_vec());
        let mut expected = CODEX.to_vec();
        expected.extend(["-m", "gpt-6-astra"]);
        assert_eq!(CodexAdapter.launch_args(&test_bot("x", Some(" gpt-6-astra ")), None), expected);
    }

    #[test]
    fn codex_resumes_with_its_subcommand() {
        let mut expected = vec!["resume", "01a143db-8973-7a12-837a-0d8dc3e76759"];
        expected.extend(CODEX);
        assert_eq!(
            CodexAdapter.launch_args(&test_bot("x", None), Some("01a143db-8973-7a12-837a-0d8dc3e76759")),
            expected
        );
    }

    #[test]
    fn codex_facts() {
        use crate::runtime::TrustPrompt;
        assert_eq!((CodexAdapter.executable(), CodexAdapter.herdr_kind()), ("codex", "codex"));
        assert_eq!(CodexAdapter.new_session_args("u-1"), None, "codex cannot take an app-chosen id");
        assert!(
            CodexAdapter.session_id_from_sbot()
                && !ClaudeAdapter.session_id_from_sbot()
                && !GrokAdapter.session_id_from_sbot()
        );
        assert!(CodexAdapter.extra_context_files().is_empty());
        assert_eq!(CodexAdapter.skill_dir(), ".agents/skills");
        assert_eq!(CodexAdapter.trust_prompt(), Some(TrustPrompt::Menu { option: "Trust and continue" }));
        assert!(CodexAdapter.rereads_context_on_resume());
        assert!(!CodexAdapter.wakes_after_background_work());
        assert!(ClaudeAdapter.wakes_after_background_work() && GrokAdapter.wakes_after_background_work());
        assert_eq!(CodexAdapter.sign_in_text(), Some("1. Sign in with ChatGPT"));
        assert_eq!((ClaudeAdapter.sign_in_text(), GrokAdapter.sign_in_text()), (None, None));
        assert_eq!(CodexAdapter.login_check(), Some(LoginCheck::ExitStatus(&["login", "status"])));
        assert_eq!(ClaudeAdapter.login_check(), Some(LoginCheck::Json(&["auth", "status", "--json"])));
        assert_eq!(GrokAdapter.login_check(), None);
    }

    #[test]
    fn codex_shows_a_background_terminal_in_its_footer() {
        let screen = "• I'll start it.\n  Worked for 9s • 10:54\n  1 background terminal running · /ps to view · /stop to close\n› Ask Codex to do anything\n";
        assert!(CodexAdapter.background_work(screen));
        assert!(CodexAdapter.background_work("  2 background terminals running · /ps to view\n› \n"));
        assert!(!CodexAdapter.background_work("  Worked for 9s • 10:55\n› Ask Codex to do anything\n"));
        assert!(
            !CodexAdapter.background_work("› Message from the user: why does it say 1 background terminal running?\n"),
            "prompt text is not codex's footer"
        );
    }

    #[test]
    fn opencode_runs_standalone_and_autonomous() {
        let bot = test_bot("x", Some(" opencode-go/kimi-k3 "));
        assert_eq!(OpencodeAdapter.launch_args(&bot, None), vec!["--standalone", "--auto"]);
        assert_eq!(OpencodeAdapter.launch_args(&bot, Some("ses_abc")), vec!["--standalone", "--auto", "-s", "ses_abc"]);
    }

    #[test]
    fn opencode_takes_an_app_chosen_id_in_its_own_form() {
        let uuid = "6f1c0f9e-2b7a-4c39-9d3e-0f5a1b2c3d4e";
        assert_eq!(OpencodeAdapter.cli_session_id(uuid), "ses_6f1c0f9e2b7a4c399d3e0f5a1b2c3d4e");
        assert_eq!(ClaudeAdapter.cli_session_id(uuid), uuid);
        assert_eq!(GrokAdapter.cli_session_id(uuid), uuid);
        assert_eq!(OpencodeAdapter.new_session_args("ses_x"), Some(vec!["-s".to_string(), "ses_x".to_string()]));
    }

    fn opencode_env(model: Option<&str>) -> (std::collections::BTreeMap<String, String>, serde_json::Value) {
        let env: std::collections::BTreeMap<_, _> =
            OpencodeAdapter.launch_env(&test_bot("x", model)).into_iter().collect();
        let config = serde_json::from_str(&env["OPENCODE_CONFIG_CONTENT"]).expect("valid JSON");
        (env, config)
    }

    #[test]
    fn opencode_launch_env_carries_its_config() {
        let (env, config) = opencode_env(None);
        assert_eq!(env.keys().collect::<Vec<_>>(), ["OPENCODE_CONFIG_CONTENT", "OPENCODE_DISABLE_AUTOUPDATE"]);
        assert_eq!(env["OPENCODE_DISABLE_AUTOUPDATE"], "1");
        assert_eq!(
            config,
            serde_json::json!({"permission": {"question": "deny"}, "websearch": {"provider": "random"}})
        );
        let (_, config) = opencode_env(Some(" opencode-go/kimi-k3 "));
        assert_eq!(config["model"], "opencode-go/kimi-k3");
        let (_, config) = opencode_env(Some(r#"a/"b\c"#));
        assert_eq!(config["model"], r#"a/"b\c"#);
        assert!(ClaudeAdapter.launch_env(&test_bot("x", Some("sonnet"))).is_empty());
    }

    #[test]
    fn opencode_facts() {
        assert_eq!((OpencodeAdapter.executable(), OpencodeAdapter.herdr_kind()), ("opencode", "opencode"));
        assert_eq!(OpencodeAdapter.provider(), Provider::Opencode);
        assert_eq!(OpencodeAdapter.skill_dir(), ".agents/skills");
        assert_eq!((OpencodeAdapter.trust_prompt(), OpencodeAdapter.sign_in_text()), (None, None));
        assert!(OpencodeAdapter.rereads_context_on_resume() && OpencodeAdapter.wakes_after_background_work());
        assert!(!OpencodeAdapter.session_id_from_sbot());
        assert!(OpencodeAdapter.extra_context_files().is_empty());
        assert_eq!(OpencodeAdapter.interrupt_keys(), ["esc", "esc"]);
        assert!(OpencodeAdapter.keeps_its_model_on_resume());
        assert!(!ClaudeAdapter.keeps_its_model_on_resume() && !CodexAdapter.keeps_its_model_on_resume());
        for adapter in [&ClaudeAdapter as &dyn ProviderAdapter, &CodexAdapter, &GrokAdapter] {
            assert_eq!(adapter.interrupt_keys(), ["esc"]);
        }
    }

    #[test]
    fn opencode_shows_background_shells_on_its_status_line() {
        let working = include_str!("../tests/fixtures/screens/opencode-working.txt");
        let idle = include_str!("../tests/fixtures/screens/opencode-idle-bg.txt");
        let echo = include_str!("../tests/fixtures/screens/opencode-multiline-echo.txt");
        assert!(OpencodeAdapter.background_work(working));
        assert!(OpencodeAdapter.background_work(idle));
        assert!(!OpencodeAdapter.background_work(echo));
        let told = "  ┃  ↓ 1 shell is what the footer says\n\n  /…/Bots/scout  8.3K (1%)  ctrl+p commands\n";
        assert!(!OpencodeAdapter.background_work(told), "only the status line counts");
        assert!(OpencodeAdapter.background_work("  /…/Bots/scout  ↓ 10 shells · 9.4K (1%)  ctrl+p commands\n"));
        assert!(!OpencodeAdapter.background_work("  /…/Bots/scout  ↓ 1 shellfish  ctrl+p commands\n"));
    }
}
