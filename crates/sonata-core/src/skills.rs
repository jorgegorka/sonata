use std::path::Path;

use crate::Refusal;

pub const SKILL_FILE: &str = "SKILL.md";
pub const MAX_NAME_LEN: usize = 64;
pub const MAX_DESCRIPTION_LEN: usize = 1024;
pub const MAX_SKILL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_SLUG_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    pub name: String,
    pub description: String,
    pub other_lines: Vec<String>,
    pub body: String,
}

pub fn parse_skill_md(text: &str) -> Result<SkillFile, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err("the file does not start with a --- line".into());
    }
    let mut front: Vec<&str> = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        front.push(line);
    }
    if !closed {
        return Err("the frontmatter has no closing --- line".into());
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    let (mut name, mut description, mut other_lines) = (None, None, Vec::new());
    let mut i = 0;
    while i < front.len() {
        let Some((key, first)) = split_key(front[i]) else {
            other_lines.push(front[i].to_string());
            i += 1;
            continue;
        };
        let mut end = i + 1;
        let mut next = i + 1;
        while let Some(line) = front.get(next) {
            if line.trim().is_empty() {
                next += 1;
            } else if line.starts_with(' ') || line.starts_with('\t') {
                next += 1;
                end = next;
            } else {
                break;
            }
        }
        match key {
            "name" => name = Some(scalar(first, &front[i + 1..end])?),
            "description" => description = Some(scalar(first, &front[i + 1..end])?),
            _ => other_lines.extend(front[i..end].iter().map(|line| line.to_string())),
        }
        i = end;
    }
    let name = name.filter(|n| !n.is_empty()).ok_or("name is missing")?;
    let description = description.filter(|d| !d.is_empty()).ok_or("description is missing")?;
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!("name is longer than {MAX_NAME_LEN} characters"));
    }
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(format!("description is longer than {MAX_DESCRIPTION_LEN} characters"));
    }
    Ok(SkillFile { name, description, other_lines, body })
}

fn split_key(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    let valid = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let rest = &line[colon + 1..];
    if !valid || !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    Some((key, rest.trim()))
}

fn scalar(first: &str, continuation: &[&str]) -> Result<String, String> {
    let lines = || continuation.iter().copied();
    match first {
        "" | ">" | ">-" | ">+" => Ok(fold(lines())),
        "|" | "|-" | "|+" => Ok(lines().map(str::trim).skip_while(|l| l.is_empty()).collect::<Vec<_>>().join("\n")),
        _ if first.starts_with('"') => {
            serde_json::from_str::<String>(first).map_err(|_| "a quoted value is not closed".to_string())
        }
        _ if first.starts_with('\'') => {
            let inner =
                first.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).ok_or("a quoted value is not closed")?;
            Ok(inner.replace("''", "'"))
        }
        _ => Ok(fold(std::iter::once(first).chain(lines()))),
    }
}

fn fold<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    let mut blanks = 0;
    for line in lines.map(str::trim) {
        if line.is_empty() {
            blanks += 1;
            continue;
        }
        if !out.is_empty() {
            if blanks == 0 {
                out.push(' ');
            } else {
                out.push_str(&"\n".repeat(blanks));
            }
        }
        out.push_str(line);
        blanks = 0;
    }
    out
}

pub fn render_skill_md(file: &SkillFile) -> String {
    let description = serde_json::to_string(&file.description).expect("a string serializes");
    let mut out = format!("---\nname: {}\ndescription: {description}\n", file.name);
    for line in &file.other_lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&file.body);
    if !file.body.is_empty() && !file.body.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn is_valid_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_SLUG_LEN
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

pub fn derive_slug(name: &str, taken: &[String]) -> String {
    let folded = deunicode::deunicode(name).to_lowercase();
    let mut base = String::new();
    for c in folded.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let mut base = base.trim_end_matches('-').to_string();
    if base.is_empty() {
        base = "skill".to_string();
    }
    base.truncate(MAX_SLUG_LEN);
    let base = base.trim_end_matches('-').to_string();
    let mut candidate = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &candidate) {
        let suffix = format!("-{n}");
        let mut stem = base.clone();
        stem.truncate(MAX_SLUG_LEN - suffix.len());
        candidate = format!("{}{suffix}", stem.trim_end_matches('-'));
        n += 1;
    }
    candidate
}

pub fn check_import(folder: &Path) -> crate::Result<SkillFile> {
    let file = folder.join(SKILL_FILE);
    if !folder.is_dir() || !file.is_file() {
        return Err(Refusal::SkillNoFile(folder.to_string_lossy().into_owned()).into());
    }
    if dir_size(folder)? > MAX_SKILL_BYTES {
        return Err(Refusal::SkillTooLarge { mb: MAX_SKILL_BYTES / (1024 * 1024) }.into());
    }
    let text = std::fs::read_to_string(&file)?;
    Ok(parse_skill_md(&text).map_err(Refusal::SkillBadFrontmatter)?)
}

pub fn dir_size(folder: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(folder)? {
        let path = entry?.path();
        let meta = std::fs::metadata(&path)?;
        total += if meta.is_dir() { dir_size(&path)? } else { meta.len() };
    }
    Ok(total)
}

pub fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    let entries =
        std::fs::read_dir(src).map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", src.display())))?;
    for entry in entries {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if std::fs::metadata(&from)?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoreError, Refusal};

    const PLAIN: &str =
        "---\nname: weekly-report\ndescription: Writes the weekly report.\n---\n\n# Weekly report\n\nSteps.\n";

    #[test]
    fn parses_plain_quoted_and_block_scalars_and_keeps_other_lines() {
        let plain = parse_skill_md(PLAIN).unwrap();
        assert_eq!((plain.name.as_str(), plain.description.as_str()), ("weekly-report", "Writes the weekly report."));
        assert_eq!(plain.body, "\n# Weekly report\n\nSteps.");
        assert!(plain.other_lines.is_empty());

        let quoted = "---\nname: \"pdf tools\"\ndescription: 'It''s for PDFs: read, fill.'\nlicense: MIT\n\
                      metadata:\n  author: me\n---\nBody\n";
        let q = parse_skill_md(quoted).unwrap();
        assert_eq!((q.name.as_str(), q.description.as_str()), ("pdf tools", "It's for PDFs: read, fill."));
        assert_eq!(
            q.other_lines,
            vec!["license: MIT".to_string(), "metadata:".to_string(), "  author: me".to_string()]
        );
        assert_eq!(q.body, "Body");

        let folded = "---\nname: long\ndescription: >\n  First line\n  second line.\n---\n";
        assert_eq!(parse_skill_md(folded).unwrap().description, "First line second line.");
        let literal = "---\nname: long\ndescription: |\n  One\n  Two\n---\n";
        assert_eq!(parse_skill_md(literal).unwrap().description, "One\nTwo");
        let continued = "---\nname: long\ndescription:\n  Starts on the next line.\n---\n";
        assert_eq!(parse_skill_md(continued).unwrap().description, "Starts on the next line.");
    }

    #[test]
    fn blank_lines_inside_a_block_description_are_kept() {
        let literal = "---\nname: x\ndescription: |\n  Para one\n\n  Para two\n\nlicense: MIT\n---\n";
        let parsed = parse_skill_md(literal).unwrap();
        assert_eq!(parsed.description, "Para one\n\nPara two");
        assert_eq!(parsed.other_lines, vec!["".to_string(), "license: MIT".to_string()]);
        let spaced = "---\nname: x\ndescription: |\n  Para one\n  \n  Para two\n---\n";
        assert_eq!(parse_skill_md(spaced).unwrap().description, "Para one\n\nPara two");
        let folded = "---\nname: x\ndescription: >\n  Para one\n  goes on.\n\n  Para two\n---\n";
        assert_eq!(parse_skill_md(folded).unwrap().description, "Para one goes on.\nPara two");
    }

    #[test]
    fn a_multi_paragraph_description_round_trips() {
        let text = "---\nname: x\ndescription: |\n  Para one\n\n  Para two\n---\nBody\n";
        let parsed = parse_skill_md(text).unwrap();
        let rendered = render_skill_md(&parsed);
        assert!(rendered.starts_with("---\nname: x\ndescription: \"Para one\\n\\nPara two\"\n---\n"), "{rendered}");
        assert_eq!(parse_skill_md(&rendered).unwrap(), parsed);
    }

    #[test]
    fn rejects_what_a_skill_cannot_do_without() {
        assert_eq!(parse_skill_md("# no frontmatter\n").unwrap_err(), "the file does not start with a --- line");
        assert_eq!(parse_skill_md("---\nname: x\n").unwrap_err(), "the frontmatter has no closing --- line");
        assert_eq!(parse_skill_md("---\ndescription: y\n---\n").unwrap_err(), "name is missing");
        assert_eq!(parse_skill_md("---\nname: x\n---\n").unwrap_err(), "description is missing");
        let long = format!("---\nname: {}\ndescription: y\n---\n", "a".repeat(65));
        assert_eq!(parse_skill_md(&long).unwrap_err(), "name is longer than 64 characters");
        let long = format!("---\nname: x\ndescription: {}\n---\n", "d".repeat(1025));
        assert_eq!(parse_skill_md(&long).unwrap_err(), "description is longer than 1024 characters");
    }

    #[test]
    fn rendering_round_trips_and_quotes_the_description() {
        let file = SkillFile {
            name: "weekly-report".into(),
            description: "Writes: \"the\" report\nin two lines".into(),
            other_lines: vec!["license: MIT".into()],
            body: "\n# Title\n".into(),
        };
        let text = render_skill_md(&file);
        let head =
            "---\nname: weekly-report\ndescription: \"Writes: \\\"the\\\" report\\nin two lines\"\nlicense: MIT\n---\n";
        assert!(text.starts_with(head), "{text}");
        assert_eq!(parse_skill_md(&text).unwrap(), SkillFile { body: "\n# Title".into(), ..file });
        let twice = parse_skill_md(&render_skill_md(&parse_skill_md(PLAIN).unwrap())).unwrap();
        assert_eq!(twice, parse_skill_md(PLAIN).unwrap());
        assert_eq!(render_skill_md(&SkillFile { body: String::new(), ..twice }).matches("---\n").count(), 2);
    }

    #[test]
    fn slugs_are_derived_folded_and_made_unique() {
        assert_eq!(derive_slug("Weekly Report", &[]), "weekly-report");
        assert_eq!(derive_slug("Résumé écrit!", &[]), "resume-ecrit");
        assert_eq!(derive_slug("???", &[]), "skill");
        assert_eq!(derive_slug("-lead-", &[]), "lead");
        let taken = vec!["weekly-report".to_string(), "weekly-report-2".to_string()];
        assert_eq!(derive_slug("weekly report", &taken), "weekly-report-3");
        assert_eq!(derive_slug(&"x".repeat(70), &[]).len(), 64);
        assert!(is_valid_slug("pdf-tools") && is_valid_slug("a1"));
        assert!(!is_valid_slug("-pdf") && !is_valid_slug("pdf-") && !is_valid_slug("Pdf") && !is_valid_slug(""));
        assert!(!is_valid_slug(&"x".repeat(65)));
    }

    #[test]
    fn an_import_is_checked_before_it_is_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("nothing");
        std::fs::create_dir(&empty).unwrap();
        assert!(matches!(check_import(&empty).unwrap_err(), CoreError::Refused(Refusal::SkillNoFile(_))));
        assert!(matches!(
            check_import(&tmp.path().join("absent")).unwrap_err(),
            CoreError::Refused(Refusal::SkillNoFile(_))
        ));

        let bad = tmp.path().join("bad");
        std::fs::create_dir(&bad).unwrap();
        std::fs::write(bad.join(SKILL_FILE), "---\nname: x\n---\n").unwrap();
        let err = check_import(&bad).unwrap_err();
        assert!(
            matches!(&err, CoreError::Refused(Refusal::SkillBadFrontmatter(reason)) if reason == "description is missing"),
            "{err}"
        );

        let big = tmp.path().join("big");
        std::fs::create_dir(&big).unwrap();
        std::fs::write(big.join(SKILL_FILE), PLAIN).unwrap();
        std::fs::File::create(big.join("data.bin")).unwrap().set_len(MAX_SKILL_BYTES + 1).unwrap();
        assert!(matches!(check_import(&big).unwrap_err(), CoreError::Refused(Refusal::SkillTooLarge { mb: 50 })));

        let good = tmp.path().join("good");
        std::fs::create_dir_all(good.join("scripts")).unwrap();
        std::fs::write(good.join(SKILL_FILE), PLAIN).unwrap();
        std::fs::write(good.join("scripts/run.sh"), "echo hi\n").unwrap();
        assert_eq!(check_import(&good).unwrap().name, "weekly-report");
        let copy = tmp.path().join("copy");
        copy_dir(&good, &copy).unwrap();
        assert_eq!(std::fs::read_to_string(copy.join("scripts/run.sh")).unwrap(), "echo hi\n");
        assert_eq!(dir_size(&copy).unwrap(), PLAIN.len() as u64 + 8);
    }

    #[test]
    fn a_copy_from_a_missing_folder_names_it() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("library/gone");
        let err = copy_dir(&gone, &tmp.path().join("copy")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(err.to_string().contains(&*gone.to_string_lossy()), "{err}");
    }
}
