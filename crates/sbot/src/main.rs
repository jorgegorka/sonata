use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use bridge_proto::{
    BotDetail, BotSummary, BridgeCommand, BridgeRequest, BridgeResponse, SkillSummary, PROTOCOL_VERSION,
};
use clap::{Args, Parser, Subcommand};

const STDIN_HINT: &str = "pass the message on stdin, e.g. sbot reply <<'EOF' ... EOF";
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Parser)]
#[command(name = "sbot", about = "Talk to the user and other bots from inside Sonata.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Submit a structured onboarding proposal for review")]
    Onboarding {
        #[command(subcommand)]
        command: OnboardingCommand,
    },
    #[command(about = "Reply to whoever sent the message you are working on. The text comes from stdin")]
    Reply {
        #[arg(help = "Attach a file; repeat for several")]
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<PathBuf>,
    },
    #[command(about = "Send a message to a teammate (@handle) or to the user (`user`). The text comes from stdin")]
    Send {
        #[arg(help = "@handle of a teammate, or `user`")]
        to: String,
        #[arg(help = "Attach a file; repeat for several")]
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<PathBuf>,
    },
    #[command(about = "List your teammates: handle, name, provider, status, and what they do")]
    Bots,
    #[command(
        about = "Ask the user a question and end your turn; the answer arrives as your next message. The text comes from stdin"
    )]
    Ask {
        #[arg(help = "Attach a file; repeat for several")]
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<PathBuf>,
        #[arg(help = "A choice the user can press to answer; repeat for up to four, short, in the user's language")]
        #[arg(long = "option", value_name = "TEXT")]
        options: Vec<String>,
    },
    #[command(about = "Hand in the result of a request you were given. The text comes from stdin")]
    Result {
        #[arg(help = "Attach a file; repeat for several")]
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<PathBuf>,
        #[arg(help = "The outcome this workflow step chose, when the step names outcomes")]
        #[arg(long = "outcome", value_name = "TEXT")]
        outcome: Option<String>,
    },
    #[command(about = "List the recurring tasks")]
    Tasks,
    #[command(about = "Create, change or delete recurring tasks")]
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    #[command(
        about = "List the workflows: each one's name, when to use it, and its steps with their bots and outcomes"
    )]
    Workflows,
    #[command(about = "Show, run, create, change or delete workflows")]
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    #[command(about = "List sessions: the open ones, or with --archived the archived ones, or with --all both")]
    Sessions {
        #[arg(help = "List the archived sessions")]
        #[arg(long, conflicts_with = "all")]
        archived: bool,
        #[arg(help = "List open and archived sessions")]
        #[arg(long)]
        all: bool,
    },
    #[command(about = "Show or archive sessions")]
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    #[command(about = "Show, create or edit bots")]
    Bot {
        #[command(subcommand)]
        command: BotCommand,
    },
    #[command(about = "List the skills in the library and which bots use them")]
    Skills,
    #[command(about = "Copy or add skills")]
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

#[derive(Subcommand)]
enum OnboardingCommand {
    #[command(about = "Submit proposal JSON from stdin; creates no workflows, bots or skills")]
    Propose {
        #[arg(long)]
        generation: String,
    },
}

#[derive(Subcommand)]
enum BotCommand {
    #[command(about = "Show a bot's role, skills and instructions")]
    Show {
        #[arg(help = "@handle of the bot")]
        handle: String,
    },
    #[command(about = "Create a bot. The instructions come from stdin")]
    Create(CreateBot),
    #[command(about = "Change a bot's instructions and skills")]
    Edit {
        #[arg(help = "@handle of the bot")]
        handle: String,
        #[arg(help = "Replace the bot's instructions with the text from stdin")]
        #[arg(long)]
        instructions: bool,
        #[arg(help = "Give the bot a skill by slug; repeat for several")]
        #[arg(long = "add-skill", value_name = "SLUG")]
        add_skills: Vec<String>,
        #[arg(help = "Take a skill from the bot by slug; repeat for several")]
        #[arg(long = "remove-skill", value_name = "SLUG")]
        remove_skills: Vec<String>,
        #[arg(
            help = "The bot's new model, as its provider's CLI names it (opencode: provider/model, e.g. opencode-go/kimi-k3). An empty one puts the bot back on the CLI's default"
        )]
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum TaskCommand {
    #[command(about = "Create a recurring task. The prompt it sends comes from stdin")]
    Create {
        #[arg(help = "The task's name")]
        #[arg(long)]
        name: String,
        #[arg(help = "@handle of the bot that runs it, in a fresh session each time")]
        #[arg(long)]
        bot: String,
        #[arg(help = "Run every this many days")]
        #[arg(long, value_name = "DAYS")]
        every: u32,
        #[arg(help = "Time of day, HH:MM, in the user's local time")]
        #[arg(long, value_name = "HH:MM")]
        at: Option<String>,
        #[arg(help = "First day, YYYY-MM-DD")]
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<String>,
        #[arg(help = "Create again the task this conversation created, after the user deleted it")]
        #[arg(long)]
        recreate: bool,
    },
    #[command(about = "Change a recurring task; what you leave out stays as it is")]
    Edit {
        #[arg(help = "Name of the task")]
        name: String,
        #[arg(help = "Rename the task")]
        #[arg(long = "name", value_name = "NAME")]
        new_name: Option<String>,
        #[arg(help = "@handle of the bot that runs it")]
        #[arg(long)]
        bot: Option<String>,
        #[arg(help = "Run every this many days")]
        #[arg(long, value_name = "DAYS")]
        every: Option<u32>,
        #[arg(help = "Time of day, HH:MM")]
        #[arg(long, value_name = "HH:MM")]
        at: Option<String>,
        #[arg(help = "First day, YYYY-MM-DD")]
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<String>,
        #[arg(help = "Turn the task on")]
        #[arg(long, conflicts_with = "off")]
        on: bool,
        #[arg(help = "Turn the task off")]
        #[arg(long)]
        off: bool,
        #[arg(help = "Replace the prompt with the text from stdin")]
        #[arg(long)]
        prompt: bool,
    },
    #[command(about = "Delete a recurring task")]
    Delete {
        #[arg(help = "Name of the task")]
        name: String,
    },
}

#[derive(Subcommand)]
enum WorkflowCommand {
    #[command(about = "Show a workflow as text, in the format create and edit read")]
    Show {
        #[arg(help = "Name of the workflow")]
        name: String,
    },
    #[command(
        about = "Start a run of a workflow. The request comes from stdin; the final result arrives as a message"
    )]
    Run {
        #[arg(help = "Name of the workflow")]
        name: String,
        #[arg(help = "Attach a file; repeat for several")]
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<PathBuf>,
    },
    #[command(about = "Create a workflow. Its definition comes from stdin, in the format show prints")]
    Create {
        #[arg(help = "Create again the workflow this conversation created, after the user deleted it")]
        #[arg(long)]
        recreate: bool,
    },
    #[command(about = "Replace a workflow with the definition from stdin; steps keep their place by name")]
    Edit {
        #[arg(help = "Name of the workflow")]
        name: String,
    },
    #[command(about = "Delete a workflow")]
    Delete {
        #[arg(help = "Name of the workflow")]
        name: String,
    },
}

#[derive(Subcommand)]
enum SessionCommand {
    #[command(about = "Show a session: its header and every message it sent or received, oldest first")]
    Show {
        #[arg(help = "The session, as @handle/number")]
        session: String,
    },
    #[command(about = "Archive sessions, with the sessions they opened")]
    Archive {
        #[arg(help = "The sessions, as @handle/number")]
        #[arg(required = true)]
        sessions: Vec<String>,
    },
}

#[derive(Args)]
struct CreateBot {
    #[arg(help = "The bot's display name")]
    #[arg(long)]
    name: String,
    #[arg(help = "The bot's color, e.g. #1a2b3c")]
    #[arg(long)]
    color: String,
    #[arg(help = "The provider CLI: claude, codex, grok or opencode")]
    #[arg(long, default_value = "")]
    provider: String,
    #[arg(
        help = "The model, as the provider's CLI names it (opencode: provider/model, e.g. opencode-go/kimi-k3). Leave it out for the CLI's default"
    )]
    #[arg(long)]
    model: Option<String>,
    #[arg(help = "Give the bot a skill by slug; repeat for several")]
    #[arg(long = "skill", value_name = "SLUG")]
    skills: Vec<String>,
    #[arg(help = "Make again the bot this conversation created, after the user deleted it")]
    #[arg(long)]
    recreate: bool,
}

#[derive(Subcommand)]
enum SkillCommand {
    #[command(about = "Copy a skill into a draft folder you can edit")]
    Copy {
        #[arg(help = "Slug of the skill in the library")]
        slug: String,
    },
    #[command(about = "Add a skill folder (with its SKILL.md) to the library")]
    Add {
        #[arg(help = "The skill's folder")]
        folder: PathBuf,
        #[arg(help = "Add again the skill this conversation added, after the user deleted it")]
        #[arg(long)]
        recreate: bool,
    },
    #[command(about = "Replace a skill in the library with the files of a folder")]
    Update {
        #[arg(help = "Slug of the skill in the library")]
        slug: String,
        #[arg(help = "The skill's folder")]
        folder: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("sbot: {message}");
            ExitCode::FAILURE
        }
    }
}

fn cli_session_id() -> Option<String> {
    cli_session_id_from(std::env::var("CODEX_THREAD_ID").ok())
}

fn cli_session_id_from(value: Option<String>) -> Option<String> {
    let value = value?;
    let id = value.trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn run(cli: Cli) -> Result<String, String> {
    let pane_id = std::env::var("HERDR_PANE_ID")
        .map_err(|_| "not running inside a Sonata bot (HERDR_PANE_ID is not set)".to_string())?;
    let socket =
        std::env::var("SBOT_SOCKET").map_err(|_| "Sonata is not running (SBOT_SOCKET is not set)".to_string())?;
    let command = match cli.command {
        Command::Onboarding { command: OnboardingCommand::Propose { generation } } => {
            BridgeCommand::OnboardingPropose { generation, body: read_body()? }
        }
        Command::Reply { files } => BridgeCommand::Reply { files: absolute(files)?, body: read_body()? },
        Command::Send { to, files } => BridgeCommand::Send { to, files: absolute(files)?, body: read_body()? },
        Command::Bots => BridgeCommand::Bots,
        Command::Ask { files, options } => BridgeCommand::Ask { files: absolute(files)?, body: read_body()?, options },
        Command::Result { files, outcome } => {
            BridgeCommand::Result { files: absolute(files)?, body: read_body()?, outcome }
        }
        Command::Bot { command } => bot_command(command, read_body)?,
        Command::Tasks => BridgeCommand::Tasks,
        Command::Task { command: TaskCommand::Create { name, bot, every, at, from, recreate } } => {
            BridgeCommand::TaskCreate { name, bot, every, at, from, prompt: read_body()?, recreate }
        }
        Command::Task { command: TaskCommand::Edit { name, new_name, bot, every, at, from, on, off, prompt } } => {
            BridgeCommand::TaskEdit {
                task: name,
                name: new_name,
                bot,
                every,
                at,
                from,
                enabled: if on {
                    Some(true)
                } else if off {
                    Some(false)
                } else {
                    None
                },
                prompt: if prompt { Some(read_body()?) } else { None },
            }
        }
        Command::Task { command: TaskCommand::Delete { name } } => BridgeCommand::TaskDelete { task: name },
        Command::Workflows => BridgeCommand::Workflows,
        Command::Workflow { command: WorkflowCommand::Show { name } } => BridgeCommand::WorkflowShow { name },
        Command::Workflow { command: WorkflowCommand::Run { name, files } } => {
            BridgeCommand::WorkflowRun { name, files: absolute(files)?, body: read_body()? }
        }
        Command::Workflow { command: WorkflowCommand::Create { recreate } } => {
            BridgeCommand::WorkflowCreate { definition: read_body()?, recreate }
        }
        Command::Workflow { command: WorkflowCommand::Edit { name } } => {
            BridgeCommand::WorkflowEdit { name, definition: read_body()? }
        }
        Command::Workflow { command: WorkflowCommand::Delete { name } } => BridgeCommand::WorkflowDelete { name },
        Command::Sessions { archived, all } => BridgeCommand::Sessions { archived, all },
        Command::Session { command: SessionCommand::Show { session } } => BridgeCommand::SessionShow { session },
        Command::Session { command: SessionCommand::Archive { sessions } } => {
            BridgeCommand::SessionArchive { sessions }
        }
        Command::Skills => BridgeCommand::Skills,
        Command::Skill { command: SkillCommand::Copy { slug } } => BridgeCommand::SkillCopy { slug },
        Command::Skill { command: SkillCommand::Add { folder, recreate } } => {
            let folder = std::path::absolute(&folder)
                .map_err(|e| format!("cannot resolve {}: {e}", folder.display()))?
                .to_string_lossy()
                .into_owned();
            BridgeCommand::SkillAdd { folder, recreate }
        }
        Command::Skill { command: SkillCommand::Update { slug, folder } } => {
            let folder = std::path::absolute(&folder)
                .map_err(|e| format!("cannot resolve {}: {e}", folder.display()))?
                .to_string_lossy()
                .into_owned();
            BridgeCommand::SkillUpdate { slug, folder }
        }
    };
    let (is_ask, is_result, is_skill_add) = match &command {
        BridgeCommand::Ask { .. } => (true, false, false),
        BridgeCommand::Result { .. } => (false, true, false),
        BridgeCommand::SkillAdd { .. } => (false, false, true),
        _ => (false, false, false),
    };
    let response =
        call(&socket, &BridgeRequest { v: PROTOCOL_VERSION, pane_id, cli_session_id: cli_session_id(), command })?;
    if let Some(error) = response.error {
        return Err(error.message);
    }
    if let Some(output) = response.output {
        return Ok(output);
    }
    if let Some(bots) = response.bots {
        return Ok(format_bots(&bots));
    }
    if let Some(bot) = response.bot {
        return Ok(format_bot_detail(&bot));
    }
    if let Some(skills) = response.skills {
        return Ok(format_skills(&skills, response.drafts_dir.as_deref()));
    }
    if let Some(draft) = response.draft {
        return Ok(if draft.resumed {
            format!("draft already at {} (resumed, not overwritten)", draft.path)
        } else {
            format!("draft at {}", draft.path)
        });
    }
    if let Some(created) = response.created {
        let n = created.name;
        return Ok(match (is_skill_add, created.already) {
            (true, false) => format!("added skill {n}"),
            (true, true) => format!("skill {n} was already added"),
            (false, false) => format!("created @{n}"),
            (false, true) => format!("@{n} was already created"),
        });
    }
    if is_ask {
        if let Some(id) = &response.message_id {
            return Ok(format!(
                "asked the user (message {id}); end your turn, the answer arrives as your next message"
            ));
        }
    }
    if is_result {
        if let (Some(id), Some(to)) = (&response.message_id, &response.to) {
            return Ok(format!("result sent to {to} (message {id})"));
        }
    }
    Ok(match (response.message_id, response.to) {
        (Some(id), Some(to)) if to == "user" => format!("sent to user (message {id})"),
        (Some(id), Some(to)) => format!("queued for {to} (message {id})"),
        (Some(id), None) => format!("sent (message {id})"),
        (None, _) => "ok".to_string(),
    })
}

fn absolute(files: Vec<PathBuf>) -> Result<Vec<String>, String> {
    files
        .into_iter()
        .map(|f| {
            std::path::absolute(&f)
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| format!("cannot resolve {}: {e}", f.display()))
        })
        .collect()
}

fn format_bots(bots: &[BotSummary]) -> String {
    if bots.is_empty() {
        return "You have no teammates yet.".to_string();
    }
    bots.iter()
        .map(|b| {
            let mut line = format!("@{} ({}) · {} · {}", b.handle, b.display_name, b.provider, b.status);
            if !b.role.is_empty() {
                line.push_str(" — ");
                line.push_str(&b.role);
            }
            if b.waiting_for_user {
                line.push_str(" · waiting for the user");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn bot_command(command: BotCommand, body: impl FnOnce() -> Result<String, String>) -> Result<BridgeCommand, String> {
    Ok(match command {
        BotCommand::Show { handle } => BridgeCommand::BotShow { handle },
        BotCommand::Create(c) => BridgeCommand::BotCreate {
            name: c.name,
            color: c.color,
            provider: c.provider,
            skills: c.skills,
            instructions: body()?,
            recreate: c.recreate,
            model: c.model,
        },
        BotCommand::Edit { handle, instructions, add_skills, remove_skills, model } => BridgeCommand::BotEdit {
            handle,
            instructions: if instructions { Some(body()?) } else { None },
            add_skills,
            remove_skills,
            model,
        },
    })
}

fn format_bot_detail(bot: &BotDetail) -> String {
    let mut out = format!("@{} ({}) · {} · {}", bot.handle, bot.display_name, bot.provider, bot.status);
    if let Some(model) = &bot.model {
        out.push_str(&format!("\nModel: {model}"));
    }
    out.push_str(&format!("\nRole: {}\nSkills:", bot.role));
    if bot.skills.is_empty() {
        out.push_str(" none");
    }
    for s in &bot.skills {
        out.push_str(&format!("\n- {} ({}): {}", s.slug, s.name, s.description));
    }
    out.push_str("\nInstructions:\n");
    out.push_str(&bot.instructions);
    out
}

fn format_skills(skills: &[SkillSummary], drafts_dir: Option<&str>) -> String {
    let mut out = if skills.is_empty() {
        "The library has no skills.".to_string()
    } else {
        skills
            .iter()
            .map(|s| {
                let used = if s.used_by.is_empty() {
                    "unused".to_string()
                } else {
                    format!("used by {}", s.used_by.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join(", "))
                };
                format!("{} — {}: {} · {used}", s.slug, s.name, s.description)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    if let Some(dir) = drafts_dir {
        out.push_str(&format!("\nDrafts for this conversation go in {dir}/<name>/."));
    }
    out
}

fn read_body() -> Result<String, String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Err(STDIN_HINT.to_string());
    }
    let mut body = String::new();
    stdin.lock().read_to_string(&mut body).map_err(|e| format!("could not read stdin: {e}"))?;
    if body.trim().is_empty() {
        return Err(format!("the message is empty; {STDIN_HINT}"));
    }
    Ok(body)
}

fn call(socket: &str, request: &BridgeRequest) -> Result<BridgeResponse, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|e| format!("Sonata is not running (could not connect to its socket: {e})"))?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(ANSWER_TIMEOUT)).map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|e| format!("could not send the message: {e}"))?;
    let mut response = String::new();
    let read = BufReader::new(stream).read_line(&mut response).map_err(|e| format!("Sonata did not answer: {e}"))?;
    if read == 0 {
        return Err("Sonata closed the connection without answering; try again".to_string());
    }
    serde_json::from_str(&response).map_err(|e| format!("unexpected answer from Sonata: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{bot_command, cli_session_id_from, format_bot_detail, format_skills, Cli, Command};
    use bridge_proto::{BotDetail, BridgeCommand, SkillSummary};
    use clap::Parser;

    #[test]
    fn onboarding_proposals_require_an_explicit_generation_id() {
        let cli = Cli::try_parse_from(["sbot", "onboarding", "propose", "--generation", "generation-1"]).unwrap();
        assert!(matches!(cli.command, Command::Onboarding {
            command: super::OnboardingCommand::Propose { generation }
        } if generation == "generation-1"));
        assert!(Cli::try_parse_from(["sbot", "onboarding", "propose"]).is_err());
    }

    fn bot_request(args: &[&str]) -> BridgeCommand {
        let cli = Cli::try_parse_from(std::iter::once("sbot").chain(args.iter().copied())).unwrap();
        let Command::Bot { command } = cli.command else { panic!("not a bot command: {args:?}") };
        bot_command(command, || Ok("Role.".to_string())).unwrap()
    }

    #[test]
    fn bot_create_and_edit_pass_the_model() {
        let create = ["bot", "create", "--name", "Scout", "--color", "#123456", "--provider", "opencode"];
        let with_model = [&create[..], &["--model", "opencode-go/kimi-k3"]].concat();
        assert_eq!(
            bot_request(&with_model),
            BridgeCommand::BotCreate {
                name: "Scout".into(),
                color: "#123456".into(),
                provider: "opencode".into(),
                skills: vec![],
                instructions: "Role.".into(),
                recreate: false,
                model: Some("opencode-go/kimi-k3".into()),
            }
        );
        assert!(matches!(bot_request(&create), BridgeCommand::BotCreate { model: None, .. }));
        let cleared = bot_request(&["bot", "edit", "@scout", "--model", ""]);
        assert_eq!(
            cleared,
            BridgeCommand::BotEdit {
                handle: "@scout".into(),
                instructions: None,
                add_skills: vec![],
                remove_skills: vec![],
                model: Some(String::new()),
            }
        );
        assert!(matches!(bot_request(&["bot", "edit", "@scout"]), BridgeCommand::BotEdit { model: None, .. }));
    }

    #[test]
    fn a_bot_detail_names_its_model() {
        let detail = BotDetail {
            handle: "scout".into(),
            display_name: "Scout".into(),
            provider: "opencode".into(),
            status: "idle".into(),
            role: "Scouts.".into(),
            instructions: "Scouts.".into(),
            skills: vec![],
            model: Some("opencode-go/kimi-k3".into()),
        };
        assert_eq!(
            format_bot_detail(&detail),
            "@scout (Scout) · opencode · idle\nModel: opencode-go/kimi-k3\nRole: Scouts.\nSkills: none\nInstructions:\nScouts."
        );
    }

    #[test]
    fn a_bot_detail_shows_role_skills_and_instructions() {
        let detail = BotDetail {
            handle: "plan".into(),
            display_name: "Plan".into(),
            provider: "claude".into(),
            status: "idle".into(),
            role: "Plans.".into(),
            instructions: "Plans.\n\nMore.".into(),
            model: None,
            skills: vec![SkillSummary {
                slug: "sonata".into(),
                name: "sonata".into(),
                description: "Sonata work.".into(),
                used_by: vec![],
            }],
        };
        assert_eq!(
            format_bot_detail(&detail),
            "@plan (Plan) · claude · idle\nRole: Plans.\nSkills:\n- sonata (sonata): Sonata work.\nInstructions:\nPlans.\n\nMore."
        );
    }

    #[test]
    fn skills_list_who_uses_them() {
        let skills = [
            SkillSummary {
                slug: "sonata-plan".into(),
                name: "sonata-plan".into(),
                description: "Plans.".into(),
                used_by: vec!["plan".into()],
            },
            SkillSummary { slug: "seo".into(), name: "SEO".into(), description: "SEO.".into(), used_by: vec![] },
        ];
        assert_eq!(
            format_skills(&skills, None),
            "sonata-plan — sonata-plan: Plans. · used by @plan\nseo — SEO: SEO. · unused"
        );
        assert_eq!(format_skills(&[], None), "The library has no skills.");
    }

    #[test]
    fn skills_end_with_where_this_conversations_drafts_go() {
        let skills =
            [SkillSummary { slug: "seo".into(), name: "SEO".into(), description: "SEO.".into(), used_by: vec![] }];
        assert_eq!(
            format_skills(&skills, Some("/bots/master/skill-drafts/session-1")),
            "seo — SEO: SEO. · unused\nDrafts for this conversation go in /bots/master/skill-drafts/session-1/<name>/."
        );
    }

    #[test]
    fn a_blank_cli_session_id_is_never_sent() {
        assert_eq!(cli_session_id_from(None), None);
        assert_eq!(cli_session_id_from(Some(String::new())), None);
        assert_eq!(cli_session_id_from(Some("  ".into())), None);
        assert_eq!(cli_session_id_from(Some(" id ".into())), Some("id".into()));
    }
}
