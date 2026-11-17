#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen<'a> {
    pub step: &'a str,
    pub handle: &'a str,
    pub visit: u32,
    pub outcome: Option<&'a str>,
    pub body: &'a str,
}

const ON_YOUR_OWN: &str =
    "Work on your own. Ask the user with `sbot ask` only what you cannot settle yourself; they answer in this chat.";

pub fn finish_line(outcomes: &[String]) -> String {
    if outcomes.is_empty() {
        return "When the step is done, send its result with `sbot result`.".to_string();
    }
    let list = outcomes.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(", ");
    format!(
        "When the step is done, send its result with `sbot result --outcome \"<outcome>\"`, where <outcome> is one \
         of: {list}. The next step depends on it."
    )
}

pub fn reminder_body(step: &str, outcomes: &[String]) -> String {
    format!("Step \"{step}\" is still open: your turn ended without its result. {}", finish_line(outcomes))
}

pub fn open_step_line(workflow: &str, step: &str, outcomes: &[String]) -> String {
    format!("You are still working on step \"{step}\" of the workflow \"{workflow}\". {}", finish_line(outcomes))
}

pub fn visit_body(request: Option<&str>, results: &[Seen], instructions: &str, outcomes: &[String]) -> String {
    let mut sections: Vec<String> = Vec::new();
    if let Some(request) = request.map(str::trim).filter(|r| !r.is_empty()) {
        sections.push(format!("Request:\n{request}"));
    }
    if !results.is_empty() {
        sections.push(format!("Results so far:\n{}", entries(results)));
    }
    let instructions = instructions.trim();
    if !instructions.is_empty() {
        sections.push(format!("Your step:\n{instructions}"));
    }
    sections.push(format!("{ON_YOUR_OWN}\n{}", finish_line(outcomes)));
    sections.join("\n\n")
}

pub fn finished_body(path: &[Seen], latest: &[Seen]) -> String {
    let path: Vec<String> = path.iter().map(|v| format!("- {}", label(v))).collect();
    format!("Path:\n{}\n\nLatest results:\n{}", path.join("\n"), entries(latest))
}

pub fn latest_results<'a>(path: &[Seen<'a>]) -> Vec<Seen<'a>> {
    let last = |i: usize, v: &Seen| !path[i + 1..].iter().any(|later| later.step == v.step);
    path.iter().enumerate().filter(|(i, v)| last(*i, v)).map(|(_, v)| *v).collect()
}

fn label(r: &Seen) -> String {
    let outcome = r.outcome.map(|o| format!(", outcome \"{o}\"")).unwrap_or_default();
    format!("Step \"{}\" (@{}), visit {}{outcome}", r.step, r.handle, r.visit)
}

fn entries(results: &[Seen]) -> String {
    results.iter().map(|r| format!("--- {}:\n{}", label(r), r.body)).collect::<Vec<_>>().join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcomes() -> Vec<String> {
        vec!["approved".into(), "changes requested".into()]
    }

    #[test]
    fn the_finish_line_names_the_outcomes_when_there_are_some() {
        assert_eq!(finish_line(&[]), "When the step is done, send its result with `sbot result`.");
        assert_eq!(
            finish_line(&outcomes()),
            "When the step is done, send its result with `sbot result --outcome \"<outcome>\"`, where <outcome> is \
             one of: \"approved\", \"changes requested\". The next step depends on it."
        );
    }

    #[test]
    fn a_reminder_names_the_step_and_how_to_finish() {
        assert_eq!(
            reminder_body("Review", &outcomes()),
            format!("Step \"Review\" is still open: your turn ended without its result. {}", finish_line(&outcomes()))
        );
        assert_eq!(
            reminder_body("Plan", &[]),
            "Step \"Plan\" is still open: your turn ended without its result. When the step is done, send its \
             result with `sbot result`."
        );
    }

    #[test]
    fn the_open_step_line_names_the_step_the_workflow_and_how_to_finish() {
        assert_eq!(
            open_step_line("Ticket to merge request", "Review", &outcomes()),
            format!(
                "You are still working on step \"Review\" of the workflow \"Ticket to merge request\". {}",
                finish_line(&outcomes())
            )
        );
        assert_eq!(
            open_step_line("Flow", "Step 1", &[]),
            "You are still working on step \"Step 1\" of the workflow \"Flow\". When the step is done, send its \
             result with `sbot result`."
        );
    }

    #[test]
    fn a_first_visit_has_the_request_the_results_and_the_step() {
        let results = [
            Seen { step: "Plan", handle: "plan", visit: 1, outcome: None, body: "The plan." },
            Seen { step: "Review", handle: "review", visit: 2, outcome: Some("changes requested"), body: "Fix x." },
        ];
        assert_eq!(
            visit_body(Some("TASK-1 build the export"), &results, "Implement it.", &outcomes()),
            format!(
                "Request:\nTASK-1 build the export\n\n\
                 Results so far:\n\
                 --- Step \"Plan\" (@plan), visit 1:\nThe plan.\n\n\
                 --- Step \"Review\" (@review), visit 2, outcome \"changes requested\":\nFix x.\n\n\
                 Your step:\nImplement it.\n\n\
                 Work on your own. Ask the user with `sbot ask` only what you cannot settle yourself; they answer in \
                 this chat.\n{}",
                finish_line(&outcomes())
            )
        );
    }

    #[test]
    fn the_finished_message_lists_the_path_then_each_steps_latest_result() {
        let path = [
            Seen { step: "Plan", handle: "plan", visit: 1, outcome: Some("check"), body: "A first plan." },
            Seen { step: "Check", handle: "review", visit: 1, outcome: None, body: "Fine." },
            Seen { step: "Plan", handle: "plan", visit: 2, outcome: Some("done"), body: "The plan." },
        ];
        let latest = latest_results(&path);
        assert_eq!(latest, vec![path[1], path[2]], "in the order the steps were last visited");
        assert_eq!(
            finished_body(&path, &latest),
            "Path:\n\
             - Step \"Plan\" (@plan), visit 1, outcome \"check\"\n\
             - Step \"Check\" (@review), visit 1\n\
             - Step \"Plan\" (@plan), visit 2, outcome \"done\"\n\n\
             Latest results:\n\
             --- Step \"Check\" (@review), visit 1:\nFine.\n\n\
             --- Step \"Plan\" (@plan), visit 2, outcome \"done\":\nThe plan."
        );
        let one = [Seen { step: "Step 1", handle: "solo", visit: 1, outcome: None, body: "Done." }];
        assert_eq!(
            finished_body(&one, &latest_results(&one)),
            "Path:\n- Step \"Step 1\" (@solo), visit 1\n\nLatest results:\n--- Step \"Step 1\" (@solo), visit 1:\nDone."
        );
    }

    #[test]
    fn a_section_with_nothing_in_it_is_left_out() {
        assert_eq!(
            visit_body(None, &[], "Do step 1.", &[]),
            "Your step:\nDo step 1.\n\n\
             Work on your own. Ask the user with `sbot ask` only what you cannot settle yourself; they answer in this \
             chat.\nWhen the step is done, send its result with `sbot result`."
        );
        assert_eq!(visit_body(Some("  \n"), &[], "Do step 1.", &[]), visit_body(None, &[], "Do step 1.", &[]));
    }
}
