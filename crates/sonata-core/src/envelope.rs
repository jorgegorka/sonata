pub fn language_name(code: &str) -> &'static str {
    match code {
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        _ => "English",
    }
}

pub const CONTINUE_LINE: &str = "The usage limit has reset. Continue where you stopped.";

pub const SCHEDULED_NOTE: &str = "This run was started by the schedule, not by the user right now. Work on your own. \
     Ask with `sbot ask` only what you cannot settle yourself, and send the outcome with `sbot reply`.";

pub const WORKFLOW_REQUEST_NOTE: &str = "The user wrote this on the Workflows page to have a new workflow made. \
     Design it with them as your playbook's §14 says under \"Designing a workflow from a description\": split it \
     into steps, pick the bot whose role and skills cover each step, and propose a new bot, a new skill, or a skill \
     for an existing bot where no bot covers a step's work. Ask only what you cannot find out. Then put one draft \
     to the user with `sbot ask`: the steps with their bots and why, and each proposal, with the options \
     \"Create all of it\" and \"Use my current bots only\" in the user's language (only \"Create the workflow\" \
     when you propose nothing). Create nothing until they approve; then add the skills, create the bots, and \
     create the workflow last.";

pub enum Origin<'a> {
    User { language: &'a str },
    Bot { handle: &'a str },
    Task { language: &'a str, name: &'a str, every_days: Option<u32>, last_run: Option<&'a str> },
    Workflow { language: &'a str, workflow: &'a str, step: &'a str, visit: u32, started_by: Option<&'a str> },
    WorkflowDone { language: &'a str, workflow: &'a str },
    WorkflowRequest { language: &'a str },
}

pub fn envelope(from: Origin, body: &str, attachments: &[String], remind: bool, owed_to: Option<&str>) -> String {
    let from_user = matches!(
        from,
        Origin::User { .. }
            | Origin::Task { .. }
            | Origin::Workflow { .. }
            | Origin::WorkflowDone { .. }
            | Origin::WorkflowRequest { .. }
    );
    let scheduled = matches!(from, Origin::Task { .. });
    let requested = matches!(from, Origin::WorkflowRequest { .. });
    let (header, reader) = match from {
        Origin::User { language } => {
            (format!("[Message from user · thread you↔user · language: {language}]"), "the user".to_string())
        }
        Origin::Bot { handle } => (format!("[Message from @{handle} · thread you↔@{handle}]"), format!("@{handle}")),
        Origin::Task { language, name, every_days, last_run } => {
            let every = match every_days {
                Some(1) => " · every day".to_string(),
                Some(n) => format!(" · every {n} days"),
                None => String::new(),
            };
            let last = last_run.map_or_else(|| "first run".to_string(), |at| format!("last run {at}"));
            (
                format!("[Recurring task \"{name}\" from the user{every} · {last} · thread you↔user · language: {language}]"),
                "the user".to_string(),
            )
        }
        Origin::Workflow { language, workflow, step, visit, started_by } => {
            let by = started_by.map_or_else(|| "the user".to_string(), |handle| format!("@{handle}"));
            (
                format!(
                    "[Workflow \"{workflow}\" · step \"{step}\" · visit {visit} · started by {by} · thread you↔user · \
                     language: {language}]"
                ),
                "the user".to_string(),
            )
        }
        Origin::WorkflowDone { language, workflow } => (
            format!("[Workflow \"{workflow}\" finished · thread you↔user · language: {language}]"),
            "the user".to_string(),
        ),
        Origin::WorkflowRequest { language } => (
            format!("[Message from user · thread you↔user · language: {language} · from the Workflows page]"),
            "the user".to_string(),
        ),
    };
    let mut text = format!("{header}\n{body}");
    if scheduled {
        text.push_str("\n\n");
        text.push_str(SCHEDULED_NOTE);
    }
    if requested {
        text.push_str("\n\n");
        text.push_str(WORKFLOW_REQUEST_NOTE);
    }
    if !attachments.is_empty() {
        text.push_str("\n\nAttachments:");
        for path in attachments {
            text.push_str("\n- `");
            text.push_str(path);
            text.push('`');
        }
    }
    if remind {
        text.push_str(&format!("\n\nReminder: {reader} only sees what you send with sbot reply."));
    }
    if let (Some(handle), true) = (owed_to, from_user) {
        text.push_str(&format!(
            "\n\nYou are still working on @{handle}'s request. When it is done, send the final result with \
             `sbot result`."
        ));
    }
    text
}

pub fn silent_turn_note(handle: &str) -> String {
    format!("(@{handle} finished without sending a reply, and its terminal showed nothing.)")
}

pub fn raw_output_body(handle: &str, text: &str) -> String {
    if text == silent_turn_note(handle) {
        return text.to_string();
    }
    format!(
        "(@{handle} ended its turn without replying with sbot. The last lines of its terminal follow; \
         they may be incomplete.)\n\n{text}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_names() {
        assert_eq!(language_name("es"), "Spanish");
        assert_eq!(language_name("fr"), "French");
        assert_eq!(language_name("de"), "German");
        assert_eq!(language_name("en"), "English");
        assert_eq!(language_name("xx"), "English");
    }

    #[test]
    fn user_envelope_format() {
        assert_eq!(
            envelope(Origin::User { language: "Spanish" }, "Hola, ¿qué tal?", &[], false, None),
            "[Message from user · thread you↔user · language: Spanish]\nHola, ¿qué tal?"
        );
        assert_eq!(
            envelope(Origin::User { language: "English" }, "hi", &[], true, None),
            "[Message from user · thread you↔user · language: English]\nhi\n\n\
             Reminder: the user only sees what you send with sbot reply."
        );
    }

    #[test]
    fn bot_envelope_format_with_attachments() {
        assert_eq!(
            envelope(Origin::Bot { handle: "writer" }, "Draft attached.", &["inbox/01J-a.md".into()], true, None),
            "[Message from @writer · thread you↔@writer]\nDraft attached.\n\nAttachments:\n- `inbox/01J-a.md`\n\n\
             Reminder: @writer only sees what you send with sbot reply."
        );
    }

    #[test]
    fn no_attachment_line_ends_in_an_image_extension() {
        let text = envelope(
            Origin::User { language: "English" },
            "Look at these.",
            &["inbox/01J-shot.png".into(), "inbox/01J-photo.JPEG".into(), "inbox/01J-anim.gif".into()],
            false,
            None,
        );
        let image = |line: &str| {
            let line = line.trim().to_ascii_lowercase();
            [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().any(|ext| line.ends_with(ext))
        };
        assert!(!text.lines().any(image), "{text}");
        assert!(text.ends_with("\n- `inbox/01J-anim.gif`"), "{text}");
    }

    #[test]
    fn a_users_message_reminds_the_bot_of_the_result_it_owes() {
        assert_eq!(
            envelope(Origin::User { language: "English" }, "Yes, go.", &[], false, Some("master")),
            "[Message from user · thread you↔user · language: English]\nYes, go.\n\n\
             You are still working on @master's request. When it is done, send the final result with `sbot result`."
        );
        assert_eq!(
            envelope(Origin::Bot { handle: "qa" }, "Tests pass.", &[], false, Some("master")),
            "[Message from @qa · thread you↔@qa]\nTests pass.",
            "a bot's message carries no such line"
        );
    }

    #[test]
    fn a_workflow_request_says_where_it_came_from_and_what_to_do() {
        let text = envelope(
            Origin::WorkflowRequest { language: "Spanish" },
            "Métricas SEO, evaluarlas y un ticket por mejora.",
            &[],
            false,
            None,
        );
        assert_eq!(
            text,
            format!(
                "[Message from user · thread you↔user · language: Spanish · from the Workflows page]\n\
                 Métricas SEO, evaluarlas y un ticket por mejora.\n\n{WORKFLOW_REQUEST_NOTE}"
            )
        );
        for part in [
            "Designing a workflow from a description",
            "\"Create all of it\"",
            "\"Use my current bots only\"",
            "Create nothing until they approve",
            "create the workflow last",
        ] {
            assert!(WORKFLOW_REQUEST_NOTE.contains(part), "{part}");
        }
    }

    #[test]
    fn a_scheduled_run_says_so() {
        let from = Origin::Task {
            language: "English",
            name: "Wiki update",
            every_days: Some(7),
            last_run: Some("2027-06-07 09:00"),
        };
        assert_eq!(
            envelope(from, "Check the repo.", &[], false, None),
            "[Recurring task \"Wiki update\" from the user · every 7 days · last run 2027-06-07 09:00 · thread you↔user · language: English]\n\
             Check the repo.\n\n\
             This run was started by the schedule, not by the user right now. Work on your own. Ask with `sbot ask` only what you cannot settle yourself, and send the outcome with `sbot reply`."
        );
        let first = Origin::Task { language: "English", name: "Daily", every_days: Some(1), last_run: None };
        assert!(envelope(first, "x", &[], false, None)
            .starts_with("[Recurring task \"Daily\" from the user · every day · first run · thread"));
        let gone = Origin::Task { language: "English", name: "Old", every_days: None, last_run: None };
        assert!(envelope(gone, "x", &[], false, None)
            .starts_with("[Recurring task \"Old\" from the user · first run · thread"));
    }

    #[test]
    fn a_scheduled_runs_note_comes_before_its_attachments_and_reminders() {
        let from = Origin::Task { language: "Spanish", name: "Review", every_days: Some(10), last_run: None };
        assert_eq!(
            envelope(from, "Review.", &["inbox/01J-a.md".into()], true, Some("master")),
            format!(
                "[Recurring task \"Review\" from the user · every 10 days · first run · thread you↔user · language: Spanish]\n\
                 Review.\n\n{SCHEDULED_NOTE}\n\nAttachments:\n- `inbox/01J-a.md`\n\n\
                 Reminder: the user only sees what you send with sbot reply.\n\n\
                 You are still working on @master's request. When it is done, send the final result with `sbot result`."
            )
        );
    }

    #[test]
    fn a_workflow_step_says_so() {
        let by_user =
            Origin::Workflow { language: "English", workflow: "Flow", step: "Review", visit: 2, started_by: None };
        assert_eq!(
            envelope(by_user, "Your step:\nReview it.", &["inbox/01J-a.md".into()], true, None),
            "[Workflow \"Flow\" · step \"Review\" · visit 2 · started by the user · thread you↔user · language: English]\n\
             Your step:\nReview it.\n\nAttachments:\n- `inbox/01J-a.md`\n\n\
             Reminder: the user only sees what you send with sbot reply."
        );
        let by_master = Origin::Workflow {
            language: "Spanish",
            workflow: "Ticket to merge request",
            step: "Plan",
            visit: 1,
            started_by: Some("master"),
        };
        assert_eq!(
            envelope(by_master, "x", &[], false, Some("master")),
            "[Workflow \"Ticket to merge request\" · step \"Plan\" · visit 1 · started by @master · thread you↔user · \
             language: Spanish]\nx\n\n\
             You are still working on @master's request. When it is done, send the final result with `sbot result`.",
            "it counts as from the user"
        );
    }

    #[test]
    fn a_finished_workflow_says_so() {
        let done = Origin::WorkflowDone { language: "French", workflow: "Ticket to merge request" };
        assert_eq!(
            envelope(done, "Path:\n- Step \"Plan\" (@plan), visit 1", &["inbox/01J-plan.md".into()], false, None),
            "[Workflow \"Ticket to merge request\" finished · thread you↔user · language: French]\n\
             Path:\n- Step \"Plan\" (@plan), visit 1\n\nAttachments:\n- `inbox/01J-plan.md`"
        );
        let done = Origin::WorkflowDone { language: "English", workflow: "Flow" };
        assert!(
            envelope(done, "x", &[], true, None)
                .ends_with("\n\nReminder: the user only sees what you send with sbot reply."),
            "it counts as from the user"
        );
    }

    #[test]
    fn raw_output_for_a_bot_says_what_it_is() {
        let body = raw_output_body("writer", "half a draft");
        assert!(body.starts_with("(@writer ended its turn without replying with sbot."), "{body}");
        assert!(body.ends_with("\n\nhalf a draft"), "{body}");
        assert_eq!(raw_output_body("writer", &silent_turn_note("writer")), silent_turn_note("writer"));
    }
}
