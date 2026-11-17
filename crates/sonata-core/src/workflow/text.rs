use std::collections::HashSet;

use crate::model::{Arrow, NewWorkflow, Step, WorkflowDef, DONE, WORKFLOW_FORMAT};

pub fn format_text(name: &str, when: &str, def: &WorkflowDef, handle_of: impl Fn(&str) -> String) -> String {
    let mut out = format!("Workflow: {}\n", name.trim());
    if !when.trim().is_empty() {
        out.push_str(&format!("When: {}\n", when.trim()));
    }
    for s in ordered(def) {
        out.push_str(&format!("\n## {}\nBot: @{}\n", s.name.trim(), handle_of(&s.bot_id)));
        for a in &s.arrows {
            out.push_str(&format!("{}\n", arrow_text(def, a)));
        }
        out.push_str(&format!("\n{}\n", s.instructions.trim()));
    }
    out
}

pub fn list_entry(name: &str, when: &str, def: &WorkflowDef, handle_of: impl Fn(&str) -> String) -> String {
    let mut out = match when.trim() {
        "" => name.trim().to_string(),
        when => format!("{}: {when}", name.trim()),
    };
    for s in ordered(def) {
        let arrows = if s.arrows.is_empty() {
            format!("-> {DONE_NAME}")
        } else {
            s.arrows.iter().map(|a| arrow_text(def, a)).collect::<Vec<_>>().join("; ")
        };
        out.push_str(&format!("\n  {} (@{}): {arrows}", s.name.trim(), handle_of(&s.bot_id)));
    }
    out
}

const DONE_NAME: &str = "Done";

fn ordered(def: &WorkflowDef) -> impl Iterator<Item = &Step> {
    let start = def.start.as_deref().and_then(|id| def.steps.iter().find(|s| s.id == id));
    start.into_iter().chain(def.steps.iter().filter(|s| Some(s.id.as_str()) != def.start.as_deref()))
}

fn arrow_text(def: &WorkflowDef, a: &Arrow) -> String {
    let target = match def.steps.iter().find(|t| t.id == a.to) {
        Some(t) if a.to != DONE => t.name.trim().to_string(),
        _ => DONE_NAME.to_string(),
    };
    match a.outcome.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        Some(o) => format!("{o} -> {target}"),
        None => format!("-> {target}"),
    }
}

struct Block {
    name: String,
    handle: String,
    arrows: Vec<(Option<String>, String)>,
    instructions: String,
}

pub fn parse_text(
    text: &str,
    bot_id_of: impl Fn(&str) -> Option<String>,
    keep: Option<&WorkflowDef>,
) -> Result<NewWorkflow, String> {
    let bad_start = || "The definition must start with \"Workflow: <name>\".".to_string();
    let mut sections: Vec<Vec<&str>> = Vec::new();
    let mut head: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            sections.push(vec![line]);
        } else {
            match sections.last_mut() {
                Some(s) => s.push(line),
                None => head.push(line),
            }
        }
    }
    let mut name = None;
    let mut when = String::new();
    for line in head.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if let Some(n) = line.strip_prefix("Workflow:") {
            name.get_or_insert_with(|| n.trim().to_string());
        } else if let Some(w) = line.strip_prefix("When:") {
            when = w.trim().to_string();
        }
    }
    let name = name.filter(|n| !n.is_empty()).ok_or_else(bad_start)?;

    let mut blocks = Vec::new();
    for lines in &sections {
        let step_name = lines[0][3..].trim().to_string();
        let no_bot = || format!("Step \"{step_name}\" needs a \"Bot: @handle\" line right after its heading.");
        let handle = lines
            .get(1)
            .and_then(|l| l.trim().strip_prefix("Bot:"))
            .map(|h| h.trim().trim_start_matches('@').to_string())
            .filter(|h| !h.is_empty())
            .ok_or_else(no_bot)?;
        let mut i = 2;
        let mut arrows = Vec::new();
        while let Some(line) = lines.get(i).map(|l| l.trim()).filter(|l| !l.is_empty() && l.contains("->")) {
            let (outcome, target) = line.split_once("->").unwrap_or((line, ""));
            let outcome = Some(outcome.trim().to_string()).filter(|o| !o.is_empty());
            arrows.push((outcome, target.trim().to_string()));
            i += 1;
        }
        let instructions = lines[i..].join("\n").trim().to_string();
        blocks.push(Block { name: step_name, handle, arrows, instructions });
    }

    let kept = |n: &str| keep.and_then(|k| k.steps.iter().find(|s| s.name.trim().eq_ignore_ascii_case(n)));
    let mut ids: Vec<Option<String>> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    for b in &blocks {
        let id = kept(&b.name).map(|s| s.id.clone()).filter(|id| used.insert(id.clone()));
        ids.push(id);
    }
    if let Some(k) = keep {
        used.extend(k.steps.iter().map(|s| s.id.clone()));
    }
    for id in ids.iter_mut().filter(|id| id.is_none()) {
        let fresh = (1..).map(|k| format!("s{k}")).find(|c| !used.contains(c)).unwrap_or_default();
        used.insert(fresh.clone());
        *id = Some(fresh);
    }
    let ids: Vec<String> = ids.into_iter().flatten().collect();

    let mut steps = Vec::new();
    for (b, id) in blocks.iter().zip(&ids) {
        let bot_id = bot_id_of(&b.handle)
            .ok_or_else(|| format!("No bot named @{}. Run `sbot bots` to see your teammates.", b.handle))?;
        let mut arrows = Vec::new();
        for (outcome, target) in &b.arrows {
            let to = if target.eq_ignore_ascii_case(DONE) {
                DONE.to_string()
            } else {
                let at = blocks.iter().position(|t| t.name.eq_ignore_ascii_case(target)).ok_or_else(|| {
                    format!("Step \"{}\": \"{target}\" is not a step of this workflow, or Done.", b.name)
                })?;
                ids[at].clone()
            };
            arrows.push(Arrow { to, outcome: outcome.clone() });
        }
        let pos = kept(&b.name).filter(|s| &s.id == id).and_then(|s| s.pos.clone());
        steps.push(Step {
            id: id.clone(),
            name: b.name.clone(),
            bot_id,
            instructions: b.instructions.clone(),
            arrows,
            pos,
        });
    }
    let def = WorkflowDef {
        version: WORKFLOW_FORMAT,
        start: ids.first().cloned(),
        steps,
        start_pos: keep.and_then(|k| k.start_pos.clone()),
        done_pos: keep.and_then(|k| k.done_pos.clone()),
    };
    Ok(NewWorkflow { name, when_to_use: when, def })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pos;

    fn arrow(to: &str, outcome: Option<&str>) -> Arrow {
        Arrow { to: to.into(), outcome: outcome.map(Into::into) }
    }

    fn st(id: &str, name: &str, bot: &str, instructions: &str, arrows: Vec<Arrow>, x: f64) -> Step {
        Step {
            id: id.into(),
            name: name.into(),
            bot_id: bot.into(),
            instructions: instructions.into(),
            arrows,
            pos: Some(Pos { x, y: 10.0 }),
        }
    }

    fn ticket() -> NewWorkflow {
        NewWorkflow {
            name: "Ticket to merge request".into(),
            when_to_use: "A request to build, from its plan to an opened merge request.".into(),
            def: WorkflowDef {
                version: WORKFLOW_FORMAT,
                start: Some("plan".into()),
                steps: vec![
                    st("plan", "Plan", "b-plan", "Write the plan.", vec![arrow("implement", None)], 1.0),
                    st(
                        "implement",
                        "Implement",
                        "b-feature",
                        "Build it.\n\nKeep it small.",
                        vec![arrow("review", None)],
                        2.0,
                    ),
                    st(
                        "review",
                        "Review",
                        "b-review",
                        "Review the change.",
                        vec![arrow("test", Some("approved")), arrow("implement", Some("changes requested"))],
                        3.0,
                    ),
                    st(
                        "test",
                        "Test",
                        "b-testing",
                        "Run the tests.",
                        vec![arrow(DONE, Some("passed")), arrow("implement", Some("failed"))],
                        4.0,
                    ),
                ],
                start_pos: Some(Pos { x: 0.0, y: 0.0 }),
                done_pos: Some(Pos { x: 9.0, y: 9.0 }),
            },
        }
    }

    fn handle_of(id: &str) -> String {
        id.trim_start_matches("b-").to_string()
    }

    fn bot_id_of(handle: &str) -> Option<String> {
        ["plan", "feature", "review", "testing"].contains(&handle).then(|| format!("b-{handle}"))
    }

    fn write(w: &NewWorkflow) -> String {
        format_text(&w.name, &w.when_to_use, &w.def, handle_of)
    }

    #[test]
    fn the_ticket_workflow_round_trips() {
        let w = ticket();
        let text = write(&w);
        assert!(text.starts_with("Workflow: Ticket to merge request\nWhen: A request"));
        assert!(text.contains(
            "## Review\nBot: @review\napproved -> Test\nchanges requested -> Implement\n\nReview the change.\n"
        ));
        assert!(text.contains("passed -> Done\n"));
        assert!(text.ends_with("Run the tests.\n") && !text.ends_with("\n\n"));
        let parsed = parse_text(&text, bot_id_of, Some(&w.def)).unwrap();
        assert_eq!(parsed, w);
        assert_eq!(write(&parsed), text);
    }

    #[test]
    fn a_list_entry_names_each_steps_bot_and_arrows() {
        let w = ticket();
        assert_eq!(
            list_entry(&w.name, &w.when_to_use, &w.def, handle_of),
            "Ticket to merge request: A request to build, from its plan to an opened merge request.\n\
             \x20 Plan (@plan): -> Implement\n\
             \x20 Implement (@feature): -> Review\n\
             \x20 Review (@review): approved -> Test; changes requested -> Implement\n\
             \x20 Test (@testing): passed -> Done; failed -> Implement"
        );
        let mut solo = w.def.clone();
        solo.steps.truncate(1);
        solo.steps[0].arrows.clear();
        assert_eq!(list_entry(" Solo ", "  ", &solo, handle_of), "Solo\n  Plan (@plan): -> Done");
    }

    #[test]
    fn instructions_keep_third_level_headings() {
        let text = "Workflow: W\n\n## One\nBot: @plan\n\nDo this.\n### Notes\nMind it.\n";
        let w = parse_text(text, bot_id_of, None).unwrap();
        assert_eq!(w.def.steps[0].instructions, "Do this.\n### Notes\nMind it.");
        assert_eq!(w.when_to_use, "");
    }

    #[test]
    fn an_edit_keeps_ids_and_positions_by_step_name() {
        let w = ticket();
        let text = write(&w).replace("Implement", "Build");
        let parsed = parse_text(&text, bot_id_of, Some(&w.def)).unwrap();
        let ids: Vec<&str> = parsed.def.steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["plan", "s1", "review", "test"]);
        assert_eq!(parsed.def.steps[1].pos, None);
        assert_eq!(parsed.def.steps[2].pos, w.def.steps[2].pos);
        assert_eq!(parsed.def.steps[0].arrows[0].to, "s1");
        assert_eq!(parsed.def.steps[2].arrows[1].to, "s1");
        let lower = text.replace("## Review", "## REVIEW");
        let again = parse_text(&lower, bot_id_of, Some(&w.def)).unwrap();
        assert_eq!(again.def.steps[2].id, "review");
    }

    #[test]
    fn parse_errors_say_what_to_fix() {
        let err = |t: &str| parse_text(t, bot_id_of, None).unwrap_err();
        assert_eq!(err("## One\nBot: @plan\n\nx"), "The definition must start with \"Workflow: <name>\".");
        assert_eq!(
            err("Workflow: W\n\n## One\n\nx"),
            "Step \"One\" needs a \"Bot: @handle\" line right after its heading."
        );
        assert_eq!(
            err("Workflow: W\n\n## One\nBot: @ghost\n\nx"),
            "No bot named @ghost. Run `sbot bots` to see your teammates."
        );
        assert_eq!(
            err("Workflow: W\n\n## One\nBot: plan\n-> Nowhere\n\nx"),
            "Step \"One\": \"Nowhere\" is not a step of this workflow, or Done."
        );
    }

    #[test]
    fn the_first_listed_step_is_the_start() {
        let text = "Workflow: W\n\n## B\nBot: plan\n-> done\n\nb\n\n## A\nBot: @review\n-> B\n\na\n";
        let w = parse_text(text, bot_id_of, None).unwrap();
        assert_eq!(w.def.start.as_deref(), Some("s1"));
        assert_eq!(w.def.steps[0].name, "B");
        assert_eq!(w.def.steps[0].arrows[0].to, DONE);
        assert_eq!(w.def.steps[1].arrows[0].to, "s1");
        let mut t = ticket();
        t.def.start = Some("review".into());
        assert_eq!(write(&t).lines().nth(3), Some("## Review"));
    }
}
