use std::collections::{HashSet, VecDeque};

use crate::model::{
    Arrow, NewWorkflow, PauseReason, Step, Visit, WorkflowDef, WorkflowRun, DONE, START, WORKFLOW_FORMAT,
};
use crate::Refusal;

pub mod prompts;
pub mod text;

pub const NAME_MAX: usize = 40;
pub const WHEN_MAX: usize = 300;
pub const STEPS_MAX: usize = 30;
pub const STEP_NAME_MAX: usize = 40;
pub const ARROWS_MAX: usize = 6;
pub const OUTCOME_MAX: usize = 60;
pub const VISIT_CAP: u32 = 3;

pub fn check_workflow(new: &NewWorkflow, bot_exists: impl Fn(&str) -> bool) -> Result<(), Refusal> {
    let def = &new.def;
    check_ids(def)?;
    if new.name.trim().is_empty() {
        return Err(Refusal::WorkflowNameRequired);
    }
    if new.name.chars().count() > NAME_MAX {
        return Err(Refusal::WorkflowNameTooLong { max: NAME_MAX });
    }
    if new.when_to_use.chars().count() > WHEN_MAX {
        return Err(Refusal::WorkflowWhenTooLong { max: WHEN_MAX });
    }
    if def.steps.is_empty() {
        return Err(Refusal::WorkflowNoSteps);
    }
    if def.steps.len() > STEPS_MAX {
        return Err(Refusal::WorkflowTooManySteps { max: STEPS_MAX });
    }
    let mut names: HashSet<String> = HashSet::new();
    for s in &def.steps {
        check_step(def, s, &mut names, &bot_exists)?;
    }
    match &def.start {
        Some(id) if step(def, id).is_some() => {}
        _ => return Err(Refusal::WorkflowNoStart),
    }
    check_reachable(def)
}

fn check_ids(def: &WorkflowDef) -> Result<(), Refusal> {
    if def.version != WORKFLOW_FORMAT {
        return Err(Refusal::WorkflowBadDefinition);
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for s in &def.steps {
        if s.id.is_empty() || s.id == START || s.id == DONE || !seen.insert(&s.id) {
            return Err(Refusal::WorkflowBadDefinition);
        }
    }
    Ok(())
}

fn check_step(
    def: &WorkflowDef,
    s: &Step,
    names: &mut HashSet<String>,
    bot_exists: &impl Fn(&str) -> bool,
) -> Result<(), Refusal> {
    let name = s.name.trim();
    if name.is_empty() {
        return Err(Refusal::WorkflowStepNameRequired);
    }
    if name.chars().count() > STEP_NAME_MAX {
        return Err(Refusal::WorkflowStepNameTooLong { step: name.to_string(), max: STEP_NAME_MAX });
    }
    if name.eq_ignore_ascii_case(DONE) {
        return Err(Refusal::WorkflowStepNameReserved);
    }
    if !names.insert(name.to_lowercase()) {
        return Err(Refusal::WorkflowStepNameTaken(name.to_string()));
    }
    let step_name = || name.to_string();
    if !bot_exists(&s.bot_id) {
        return Err(Refusal::WorkflowStepNoBot { step: step_name() });
    }
    if s.instructions.trim().is_empty() {
        return Err(Refusal::WorkflowStepInstructionsRequired { step: step_name() });
    }
    for a in &s.arrows {
        if a.to != DONE && step(def, &a.to).is_none() {
            return Err(Refusal::WorkflowArrowTarget { step: step_name() });
        }
    }
    if s.arrows.len() > 1 {
        let mut seen: HashSet<String> = HashSet::new();
        for a in &s.arrows {
            let outcome = a.outcome.as_deref().map(str::trim).unwrap_or("");
            if outcome.is_empty() {
                return Err(Refusal::WorkflowOutcomeRequired { step: step_name() });
            }
            if outcome.chars().count() > OUTCOME_MAX {
                return Err(Refusal::WorkflowOutcomeTooLong { step: step_name(), max: OUTCOME_MAX });
            }
            if outcome.contains(['\n', '\r']) {
                return Err(Refusal::WorkflowOutcomeLine { step: step_name() });
            }
            if !seen.insert(outcome.to_lowercase()) {
                return Err(Refusal::WorkflowOutcomeTaken { step: step_name(), outcome: outcome.to_string() });
            }
        }
    }
    if s.arrows.len() > ARROWS_MAX {
        return Err(Refusal::WorkflowTooManyArrows { step: step_name(), max: ARROWS_MAX });
    }
    Ok(())
}

fn check_reachable(def: &WorkflowDef) -> Result<(), Refusal> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&Step> = VecDeque::new();
    if let Some(first) = def.start.as_deref().and_then(|id| step(def, id)) {
        seen.insert(&first.id);
        queue.push_back(first);
    }
    while let Some(s) = queue.pop_front() {
        for a in &s.arrows {
            if let Some(next) = step(def, &a.to) {
                if seen.insert(&next.id) {
                    queue.push_back(next);
                }
            }
        }
    }
    match def.steps.iter().find(|s| !seen.contains(s.id.as_str())) {
        Some(s) => Err(Refusal::WorkflowUnreachable { step: s.name.trim().to_string() }),
        None => Ok(()),
    }
}

pub fn step<'a>(def: &'a WorkflowDef, id: &str) -> Option<&'a Step> {
    def.steps.iter().find(|s| s.id == id)
}

pub fn outcomes(step: &Step) -> Vec<String> {
    if step.arrows.len() < 2 {
        return Vec::new();
    }
    step.arrows.iter().map(|a| a.outcome.as_deref().unwrap_or("").trim().to_string()).collect()
}

pub fn follow<'a>(step: &'a Step, outcome: Option<&str>) -> Result<Option<&'a Arrow>, String> {
    match step.arrows.len() {
        0 => return Ok(None),
        1 => return Ok(step.arrows.first()),
        _ => {}
    }
    let names = outcomes(step);
    let list = names.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(", ");
    let name = step.name.trim();
    let Some(given) = outcome.map(str::trim).filter(|o| !o.is_empty()) else {
        return Err(format!("Step \"{name}\" has several outcomes. Add --outcome with one of: {list}."));
    };
    match step.arrows.iter().zip(&names).find(|(_, o)| o.eq_ignore_ascii_case(given)) {
        Some((arrow, _)) => Ok(Some(arrow)),
        None => Err(format!("\"{given}\" is not an outcome of step \"{name}\". Use one of: {list}.")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Visit { step_id: String, n: u32 },
    Pause(PauseReason),
    Finish,
}

pub fn decide(run: &WorkflowRun, visits: &[Visit], to: Option<&str>, bot_exists: impl Fn(&str) -> bool) -> Decision {
    let Some(to) = to.filter(|to| *to != DONE) else { return Decision::Finish };
    let made = visits.iter().filter(|v| v.step_id == to).count() as u32;
    if made >= VISIT_CAP + run.extra_visits.get(to).copied().unwrap_or(0) {
        return Decision::Pause(PauseReason::VisitCap { step_id: to.to_string() });
    }
    match step(&run.def, to) {
        Some(s) if bot_exists(&s.bot_id) => Decision::Visit { step_id: to.to_string(), n: made + 1 },
        _ => Decision::Pause(PauseReason::BotMissing { step_id: to.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Arrow, RunStatus};

    fn arrow(to: &str, outcome: Option<&str>) -> Arrow {
        Arrow { to: to.into(), outcome: outcome.map(Into::into) }
    }

    fn st(id: &str, name: &str, bot: &str, arrows: Vec<Arrow>) -> Step {
        Step {
            automatic_child_review: false,
            id: id.into(),
            name: name.into(),
            bot_id: bot.into(),
            instructions: "Do it.".into(),
            arrows,
            pos: None,
        }
    }

    fn ticket() -> NewWorkflow {
        NewWorkflow {
            name: "Ticket to merge request".into(),
            when_to_use: "A ticket to build.".into(),
            def: WorkflowDef {
                version: WORKFLOW_FORMAT,
                start: Some("plan".into()),
                steps: vec![
                    st("plan", "Plan", "b-plan", vec![arrow("implement", None)]),
                    st("implement", "Implement", "b-feature", vec![arrow("review", None)]),
                    st(
                        "review",
                        "Review",
                        "b-review",
                        vec![arrow("test", Some("approved")), arrow("implement", Some("changes requested"))],
                    ),
                    st(
                        "test",
                        "Test",
                        "b-testing",
                        vec![arrow(DONE, Some("passed")), arrow("implement", Some("failed"))],
                    ),
                ],
                start_pos: None,
                done_pos: None,
            },
        }
    }

    fn review(w: &mut NewWorkflow) -> &mut Step {
        &mut w.def.steps[2]
    }

    #[test]
    fn the_ticket_workflow_passes() {
        assert_eq!(check_workflow(&ticket(), |_| true), Ok(()));
    }

    #[test]
    fn each_rule_has_its_refusal() {
        type Change = Box<dyn Fn(&mut NewWorkflow)>;
        let cases: Vec<(Change, Refusal)> = vec![
            (Box::new(|w| w.name = "".into()), Refusal::WorkflowNameRequired),
            (Box::new(|w| w.name = "x".repeat(41)), Refusal::WorkflowNameTooLong { max: 40 }),
            (Box::new(|w| w.when_to_use = "x".repeat(301)), Refusal::WorkflowWhenTooLong { max: 300 }),
            (Box::new(|w| w.def.steps.clear()), Refusal::WorkflowNoSteps),
            (
                Box::new(|w| {
                    for i in 0..27 {
                        w.def.steps.push(st(&format!("x{i}"), &format!("X{i}"), "b", vec![]));
                    }
                }),
                Refusal::WorkflowTooManySteps { max: 30 },
            ),
            (Box::new(|w| w.def.steps[0].name = "".into()), Refusal::WorkflowStepNameRequired),
            (
                Box::new(|w| w.def.steps[0].name = "x".repeat(41)),
                Refusal::WorkflowStepNameTooLong { step: "x".repeat(41), max: 40 },
            ),
            (Box::new(|w| w.def.steps[0].name = "DONE".into()), Refusal::WorkflowStepNameReserved),
            (Box::new(|w| w.def.steps[1].name = "PLAN".into()), Refusal::WorkflowStepNameTaken("PLAN".into())),
            (Box::new(|w| review(w).bot_id = "missing".into()), Refusal::WorkflowStepNoBot { step: "Review".into() }),
            (
                Box::new(|w| review(w).instructions = "  ".into()),
                Refusal::WorkflowStepInstructionsRequired { step: "Review".into() },
            ),
            (Box::new(|w| w.def.start = None), Refusal::WorkflowNoStart),
            (Box::new(|w| w.def.start = Some("nope".into())), Refusal::WorkflowNoStart),
            (
                Box::new(|w| review(w).arrows[0].to = "nope".into()),
                Refusal::WorkflowArrowTarget { step: "Review".into() },
            ),
            (
                Box::new(|w| review(w).arrows[1].outcome = None),
                Refusal::WorkflowOutcomeRequired { step: "Review".into() },
            ),
            (
                Box::new(|w| review(w).arrows[0].outcome = Some("x".repeat(61))),
                Refusal::WorkflowOutcomeTooLong { step: "Review".into(), max: 60 },
            ),
            (
                Box::new(|w| review(w).arrows[0].outcome = Some("a\nb".into())),
                Refusal::WorkflowOutcomeLine { step: "Review".into() },
            ),
            (
                Box::new(|w| {
                    review(w).arrows[0].outcome = Some("Approved".into());
                    review(w).arrows[1].outcome = Some("approved".into());
                }),
                Refusal::WorkflowOutcomeTaken { step: "Review".into(), outcome: "approved".into() },
            ),
            (
                Box::new(|w| {
                    review(w).arrows = (0..7).map(|i| arrow("implement", Some(&format!("o{i}")))).collect();
                }),
                Refusal::WorkflowTooManyArrows { step: "Review".into(), max: 6 },
            ),
            (
                Box::new(|w| w.def.steps.push(st("lonely", "Lonely", "b", vec![]))),
                Refusal::WorkflowUnreachable { step: "Lonely".into() },
            ),
            (Box::new(|w| w.def.version = 2), Refusal::WorkflowBadDefinition),
            (Box::new(|w| w.def.steps[1].id = "plan".into()), Refusal::WorkflowBadDefinition),
            (Box::new(|w| w.def.steps[1].id = "done".into()), Refusal::WorkflowBadDefinition),
            (Box::new(|w| w.def.steps[1].id = "".into()), Refusal::WorkflowBadDefinition),
        ];
        for (i, (change, expected)) in cases.into_iter().enumerate() {
            let mut w = ticket();
            change(&mut w);
            assert_eq!(check_workflow(&w, |id| id != "missing"), Err(expected), "case {i}");
        }
    }

    #[test]
    fn follow_picks_the_named_arrow() {
        let w = ticket();
        let review = step(&w.def, "review").unwrap();
        assert_eq!(follow(review, Some(" APPROVED ")).unwrap().unwrap().to, "test");
        let missing = follow(review, None).unwrap_err();
        assert_eq!(
            missing,
            "Step \"Review\" has several outcomes. Add --outcome with one of: \"approved\", \"changes requested\"."
        );
        assert_eq!(
            follow(review, Some("maybe")).unwrap_err(),
            "\"maybe\" is not an outcome of step \"Review\". Use one of: \"approved\", \"changes requested\"."
        );
        let plan = step(&w.def, "plan").unwrap();
        assert_eq!(follow(plan, Some("whatever")).unwrap().unwrap().to, "implement");
        assert_eq!(follow(&st("e", "End", "b", vec![]), None), Ok(None));
        assert_eq!(outcomes(review), vec!["approved", "changes requested"]);
        assert!(outcomes(plan).is_empty());
    }

    fn run_of(w: NewWorkflow) -> WorkflowRun {
        WorkflowRun {
            project_id: "default".into(),
            id: "run".into(),
            workflow_id: None,
            name: w.name,
            def: w.def,
            request: "TASK-1".into(),
            request_files: vec![],
            opener_session: None,
            root_session: None,
            status: RunStatus::Running,
            pause: None,
            extra_visits: Default::default(),
            created_at: String::new(),
            ended_at: None,
        }
    }

    fn visits(steps: &[&str]) -> Vec<Visit> {
        let mut n: std::collections::HashMap<&str, u32> = Default::default();
        steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let count = n.entry(step).or_default();
                *count += 1;
                Visit {
                    id: format!("v{i}"),
                    run_id: "run".into(),
                    ord: i as i64 + 1,
                    step_id: step.to_string(),
                    n: *count,
                    session_id: None,
                    request_message: format!("m{i}"),
                    result_message: None,
                    outcome: None,
                    reminded: false,
                    started_at: String::new(),
                    ended_at: None,
                }
            })
            .collect()
    }

    fn to_visit(step: &str, n: u32) -> Decision {
        Decision::Visit { step_id: step.into(), n }
    }

    #[test]
    fn decide_visits_the_target_with_its_next_number() {
        let run = run_of(ticket());
        let so_far = visits(&["plan", "implement", "review"]);
        assert_eq!(decide(&run, &so_far, Some("test"), |_| true), to_visit("test", 1));
        assert_eq!(decide(&run, &so_far, Some("implement"), |_| true), to_visit("implement", 2));
    }

    #[test]
    fn decide_finishes_at_done_or_without_an_arrow() {
        let run = run_of(ticket());
        let so_far = visits(&["plan"]);
        assert_eq!(decide(&run, &so_far, None, |_| true), Decision::Finish);
        assert_eq!(decide(&run, &so_far, Some(DONE), |_| false), Decision::Finish);
    }

    #[test]
    fn decide_pauses_past_the_cap_unless_another_round_was_allowed() {
        let mut run = run_of(ticket());
        let so_far = visits(&["plan", "implement", "review", "implement", "review", "implement", "review"]);
        let cap = Decision::Pause(PauseReason::VisitCap { step_id: "implement".into() });
        assert_eq!(decide(&run, &so_far, Some("implement"), |_| true), cap);
        assert_eq!(decide(&run, &so_far, Some("implement"), |_| false), cap);
        run.extra_visits.insert("implement".into(), 1);
        assert_eq!(decide(&run, &so_far, Some("implement"), |_| true), to_visit("implement", 4));
        assert_eq!(
            decide(&run, &so_far, Some("review"), |_| true),
            Decision::Pause(PauseReason::VisitCap { step_id: "review".into() })
        );
    }

    #[test]
    fn decide_pauses_when_the_steps_bot_is_gone() {
        let run = run_of(ticket());
        let so_far = visits(&["plan", "implement", "review"]);
        assert_eq!(
            decide(&run, &so_far, Some("test"), |id| id != "b-testing"),
            Decision::Pause(PauseReason::BotMissing { step_id: "test".into() })
        );
    }
}
