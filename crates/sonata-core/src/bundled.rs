use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct MasterBot {
    pub display_name: &'static str,
    pub handle: &'static str,
    pub color: &'static str,
    pub role_line: &'static str,
    pub skill_slug: &'static str,
    pub skill_description: &'static str,
}

pub const MASTER: MasterBot = MasterBot {
    display_name: "Master",
    handle: "master",
    color: "#4f46e5",
    role_line: "Coordinates your bots, checks their results, and helps create bots, skills and workflows.",
    skill_slug: "sonata-master",
    skill_description: "Coordinate work, discover available bots, propose helpers, and report verified results.",
};
const MASTER_PLAYBOOK: &str = include_str!("../bundled/master/playbook.md");
pub const NEW_ASK: &str = "ask with `sbot ask` and end the turn";

pub fn master_skill_body() -> String {
    MASTER_PLAYBOOK.to_string()
}

pub fn master_instructions(project: Option<&Path>) -> String {
    format!(
        "{role}\n\nYou are the orchestrator of the Sonata bots. Read your playbook \
         `.claude/skills/{slug}/SKILL.md` (also available in `.agents/skills/{slug}/SKILL.md`) \
         and follow it for every message. Discover the bots and skills that actually exist.\n\n\
         {project}\n\nThis pane has no AskUserQuestion tool: when you need the user's answer or approval, \
         {ask}; the answer arrives as your next message.",
        role = MASTER.role_line,
        slug = MASTER.skill_slug,
        project = checkout_paragraph(project),
        ask = NEW_ASK,
    )
}

pub fn checkout_paragraph(project: Option<&Path>) -> String {
    match project {
        Some(path) => format!(
            "The project folder is at `{}`. Work in this folder when the request concerns this project. \
             Check its instructions and available tools before working. Keep durable notes in MEMORY.md; \
             the skill copies in your bot folder are regenerated when a session starts.",
            path.display()
        ),
        None => "The project folder is not set. When a request needs files or a repository, ask the user for \
                 the absolute path and record it in MEMORY.md. Other requests do not need a project folder."
            .to_string(),
    }
}

pub fn find_checkout_paragraph(instructions: &str) -> Option<(Range<usize>, Option<PathBuf>)> {
    let unset = checkout_paragraph(None);
    if let Some(start) = instructions.find(&unset) {
        return Some((start..start + unset.len(), None));
    }
    let template = checkout_paragraph(Some(Path::new("\0")));
    let (head, tail) = template.split_once('\0')?;
    let start = instructions.find(head)?;
    let path_start = start + head.len();
    let path_end = path_start + instructions[path_start..].find(tail)?;
    let path = &instructions[path_start..path_end];
    if path.is_empty() || path.contains('\n') {
        return None;
    }
    Some((start..path_end + tail.len(), Some(PathBuf::from(path))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_paragraph_round_trips_and_preserves_user_edits() {
        for path in [None, Some(Path::new("/Users/example/My Project"))] {
            let paragraph = checkout_paragraph(path);
            let text = format!("Custom instructions.\n\n{paragraph}\n\nOther notes.");
            let (range, found) = find_checkout_paragraph(&text).unwrap();
            assert_eq!(&text[range], paragraph);
            assert_eq!(found.as_deref(), path);
            assert!(find_checkout_paragraph(&text.replace("project folder", "custom workspace")).is_none());
        }
    }

    #[test]
    fn master_has_a_standalone_playbook_and_valid_identity() {
        assert!(crate::skills::is_valid_slug(MASTER.skill_slug));
        assert_eq!(crate::handle::derive_handle(MASTER.display_name, &[]), MASTER.handle);
        let body = master_skill_body();
        for command in [
            "sbot bots",
            "sbot skills",
            "sbot ask",
            "sbot send",
            "sbot result",
            "sbot bot create",
            "sbot task create",
            "sbot tasks",
            "sbot task edit",
            "sbot task delete",
            "sbot sessions",
            "sbot session show",
            "sbot session archive",
            "sbot skill update",
            "sbot bot edit",
            "sbot workflow run",
        ] {
            assert!(body.contains(command), "missing command: {command}");
        }
        assert!(body.contains("after the user approves"));
    }
}
