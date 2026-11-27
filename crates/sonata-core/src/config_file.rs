use crate::model::{Arrow, Pos};
use crate::skills::{is_valid_slug, parse_skill_md, SkillFile, MAX_SKILL_BYTES, SKILL_FILE};
use crate::{CoreError, Refusal};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub const FORMAT: &str = "sonata-config";
pub const VERSION: u32 = 1;
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigFile {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub app_version: String,
    #[serde(default)]
    pub bots: Vec<FileBot>,
    #[serde(default)]
    pub skills: Vec<FileSkill>,
    #[serde(default)]
    pub workflows: Vec<FileWorkflow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileBot {
    pub handle: String,
    pub name: String,
    pub color: String,
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub checkout: bool,
    #[serde(default)]
    pub master: bool,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileSkill {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileWorkflow {
    pub name: String,
    #[serde(default)]
    pub when_to_use: String,
    pub definition: FileDef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDef {
    pub version: u32,
    pub start: Option<String>,
    #[serde(default)]
    pub start_pos: Option<Pos>,
    #[serde(default)]
    pub done_pos: Option<Pos>,
    #[serde(default)]
    pub steps: Vec<FileStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileStep {
    #[serde(default)]
    pub automatic_child_review: bool,
    pub id: String,
    pub name: String,
    pub bot: Option<String>,
    #[serde(default)]
    pub pos: Option<Pos>,
    #[serde(default)]
    pub arrows: Vec<Arrow>,
    #[serde(default)]
    pub instructions: String,
}

pub fn to_yaml(file: &ConfigFile) -> crate::Result<String> {
    serde_yaml_ng::to_string(file).map_err(|e| CoreError::Internal(format!("configuration file: {e}")))
}

fn not_yaml(err: serde_yaml_ng::Error) -> CoreError {
    let line = err.location().map(|l| l.line()).unwrap_or(1);
    Refusal::ConfigFileNotYaml { line, reason: err.to_string() }.into()
}

pub fn from_yaml(text: &str) -> crate::Result<ConfigFile> {
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).map_err(not_yaml)?;
    let Some(map) = value.as_mapping() else {
        return Err(Refusal::ConfigFileWrongFormat.into());
    };
    if map.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        return Err(Refusal::ConfigFileWrongFormat.into());
    }
    if let Some(version) = map.get("version").and_then(|v| v.as_u64()) {
        if version > u64::from(VERSION) {
            return Err(Refusal::ConfigFileTooNew { version: u32::try_from(version).unwrap_or(u32::MAX) }.into());
        }
    }
    serde_yaml_ng::from_str(text).map_err(not_yaml)
}

pub fn read_file(path: &Path) -> crate::Result<(ConfigFile, [u8; 32])> {
    if std::fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Err(Refusal::ConfigFileTooLarge { mb: MAX_FILE_BYTES / (1024 * 1024) }.into());
    }
    let bytes = std::fs::read(path)?;
    let hash: [u8; 32] = Sha256::digest(&bytes).into();
    let text = String::from_utf8(bytes)
        .map_err(|e| Refusal::ConfigFileNotYaml { line: 1, reason: e.utf8_error().to_string() })?;
    Ok((from_yaml(&text)?, hash))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub path: String,
    pub bytes: Vec<u8>,
    pub executable: bool,
}

pub struct DecodedSkill {
    pub blobs: Vec<Blob>,
    pub file: SkillFile,
}

const DS_STORE: &str = ".DS_Store";

fn naming(path: &Path) -> impl Fn(std::io::Error) -> std::io::Error + '_ {
    move |e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

fn each_file(
    dir: &Path,
    prefix: &str,
    visit: &mut impl FnMut(String, &Path, &std::fs::Metadata) -> std::io::Result<()>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir).map_err(naming(dir))? {
        let entry = entry.map_err(naming(dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == DS_STORE {
            continue;
        }
        let full = entry.path();
        let meta = std::fs::symlink_metadata(&full).map_err(naming(&full))?;
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if meta.is_dir() {
            each_file(&full, &path, visit)?;
        } else if meta.is_file() {
            visit(path, &full, &meta)?;
        }
    }
    Ok(())
}

pub fn read_skill_folder(dir: &Path) -> std::io::Result<Vec<Blob>> {
    let mut out = Vec::new();
    each_file(dir, "", &mut |path, full, meta| {
        let bytes = std::fs::read(full).map_err(naming(full))?;
        out.push(Blob { path, bytes, executable: meta.permissions().mode() & 0o111 != 0 });
        Ok(())
    })?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn folder_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    each_file(dir, "", &mut |_, _, meta| {
        total += meta.len();
        Ok(())
    })?;
    Ok(total)
}

pub fn to_entry(blob: &Blob) -> FileEntry {
    let (text, base64) = match std::str::from_utf8(&blob.bytes) {
        Ok(text) => (Some(text.to_string()), None),
        Err(_) => (None, Some(STANDARD.encode(&blob.bytes))),
    };
    FileEntry { path: blob.path.clone(), executable: blob.executable, text, base64 }
}

fn path_is_valid(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains('\0')
        && path.split('/').all(|part| !part.is_empty() && !part.chars().all(|c| c == '.'))
}

pub fn decode_skill(skill: &FileSkill) -> Result<DecodedSkill, Refusal> {
    let slug = &skill.slug;
    if !is_valid_slug(slug) {
        return Err(Refusal::SkillSlugInvalid(slug.clone()));
    }
    let mut blobs = Vec::new();
    for entry in &skill.files {
        let path = &entry.path;
        if !path_is_valid(path) {
            return Err(Refusal::SkillPathInvalid(path.clone()));
        }
        let bytes = match (&entry.text, &entry.base64) {
            (Some(text), None) => text.as_bytes().to_vec(),
            (None, Some(data)) => STANDARD.decode(data).map_err(|_| Refusal::SkillFileContent(path.clone()))?,
            _ => return Err(Refusal::SkillFileContent(path.clone())),
        };
        if path.rsplit('/').next() == Some(DS_STORE) {
            continue;
        }
        blobs.push(Blob { path: path.clone(), bytes, executable: entry.executable });
    }
    let mut seen = HashSet::new();
    for blob in &blobs {
        if !seen.insert(blob.path.to_lowercase()) {
            return Err(Refusal::SkillPathRepeated(blob.path.clone()));
        }
    }
    for blob in &blobs {
        let folder = format!("{}/", blob.path.to_lowercase());
        if seen.iter().any(|other| other.starts_with(&folder)) {
            return Err(Refusal::SkillPathConflict(blob.path.clone()));
        }
    }
    let Some(skill_md) = blobs.iter().find(|b| b.path == SKILL_FILE) else {
        return Err(Refusal::SkillNoFile(slug.clone()));
    };
    let text = std::str::from_utf8(&skill_md.bytes)
        .map_err(|_| Refusal::SkillBadFrontmatter("the file is not UTF-8 text".into()))?;
    let file = parse_skill_md(text).map_err(Refusal::SkillBadFrontmatter)?;
    if file.name != *slug {
        return Err(Refusal::SkillNameNotSlug(slug.clone()));
    }
    let total: u64 = blobs.iter().map(|b| b.bytes.len() as u64).sum();
    if total > MAX_SKILL_BYTES {
        return Err(Refusal::SkillTooLarge { mb: MAX_SKILL_BYTES / (1024 * 1024) });
    }
    blobs.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(DecodedSkill { blobs, file })
}

pub fn write_blobs(dir: &Path, blobs: &[Blob]) -> std::io::Result<()> {
    for blob in blobs {
        let path = dir.join(&blob.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &blob.bytes)?;
        if blob.executable {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refusal::Refusal;
    use crate::CoreError;
    use std::time::{Duration, Instant};

    fn header() -> ConfigFile {
        ConfigFile {
            format: FORMAT.into(),
            version: VERSION,
            exported_at: "2026-11-13T10:00:00+01:00".into(),
            app_version: "1.7.0".into(),
            bots: vec![],
            skills: vec![],
            workflows: vec![],
        }
    }

    fn bot(instructions: &str) -> FileBot {
        FileBot {
            handle: "plan".into(),
            name: "Plan".into(),
            color: "blue".into(),
            provider: "claude".into(),
            model: None,
            checkout: false,
            master: false,
            skills: vec![],
            instructions: instructions.into(),
        }
    }

    const HEADER_YAML: &str = "format: sonata-config\nversion: 1\nexported_at: now\napp_version: 1.7.0\n";

    #[test]
    fn awkward_text_reads_back_byte_for_byte() {
        let samples = [
            "trailing  \nspaces\t\n",
            "\n\nleading blank lines",
            "a\r\nb",
            "# not a comment",
            "key: value",
            "---\nlooks like a document\n...",
            "emoji 🧪 ñ",
            "",
            "\n",
            "  indented first",
        ];
        let mut file = header();
        for sample in samples {
            file.bots.push(bot(sample));
        }
        file.skills.push(FileSkill {
            slug: "s".into(),
            name: "S".into(),
            description: String::new(),
            files: samples
                .iter()
                .enumerate()
                .map(|(i, s)| FileEntry {
                    path: format!("f{i}.md"),
                    executable: false,
                    text: Some((*s).into()),
                    base64: None,
                })
                .collect(),
        });
        let yaml = to_yaml(&file).unwrap();
        assert_eq!(from_yaml(&yaml).unwrap(), file);
    }

    #[test]
    fn multi_line_text_is_written_as_a_block() {
        let mut file = header();
        file.bots.push(bot("You plan Sonata tickets.\nSecond line.\n"));
        let yaml = to_yaml(&file).unwrap();
        assert!(yaml.contains("instructions: |"), "{yaml}");
    }

    #[test]
    fn typed_fields_keep_yaml_words_as_text() {
        let yaml =
            format!("{HEADER_YAML}bots:\n- handle: on\n  name: no\n  color: blue\n  provider: claude\n  model: 1.10\n");
        let file = from_yaml(&yaml).unwrap();
        assert_eq!(file.bots[0].name, "no");
        assert_eq!(file.bots[0].model.as_deref(), Some("1.10"));
        assert_eq!(file.bots[0].handle, "on");
    }

    fn refused(result: crate::Result<ConfigFile>) -> Refusal {
        match result {
            Err(CoreError::Refused(r)) => r,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn another_format_or_a_newer_version_is_refused() {
        assert!(matches!(refused(from_yaml("format: other\nversion: 1")), Refusal::ConfigFileWrongFormat));
        assert!(matches!(refused(from_yaml("- a\n- b")), Refusal::ConfigFileWrongFormat));
        assert!(matches!(
            refused(from_yaml("format: sonata-config\nversion: 2")),
            Refusal::ConfigFileTooNew { version: 2 }
        ));
    }

    #[test]
    fn bad_yaml_names_the_line() {
        let yaml = format!("{HEADER_YAML}bots:\n  - handle: a\n   name: b\n");
        match refused(from_yaml(&yaml)) {
            Refusal::ConfigFileNotYaml { line, .. } => assert_eq!(line, 7, "{yaml}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_alias_bomb_is_refused_quickly() {
        let mut yaml = String::from(HEADER_YAML);
        yaml.push_str("bomb0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..=9 {
            let refs = vec![format!("*a{}", i - 1); 10].join(", ");
            yaml.push_str(&format!("bomb{i}: &a{i} [{refs}]\n"));
        }
        let started = Instant::now();
        let result = from_yaml(&yaml);
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
        assert!(matches!(refused(result), Refusal::ConfigFileNotYaml { .. }));
    }

    #[test]
    fn a_provider_this_app_does_not_know_still_reads() {
        let yaml = format!("{HEADER_YAML}bots:\n- handle: g\n  name: G\n  color: blue\n  provider: gemini\n");
        assert_eq!(from_yaml(&yaml).unwrap().bots[0].provider, "gemini");
    }

    #[test]
    fn a_file_over_the_limit_is_refused_before_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.yaml");
        std::fs::File::create(&path).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(matches!(read_file(&path), Err(CoreError::Refused(Refusal::ConfigFileTooLarge { mb: 256 }))));
    }

    fn entry(path: &str, text: &str) -> FileEntry {
        FileEntry { path: path.into(), executable: false, text: Some(text.into()), base64: None }
    }

    fn skill(slug: &str, files: Vec<FileEntry>) -> FileSkill {
        FileSkill { slug: slug.into(), name: slug.into(), description: String::new(), files }
    }

    const DEMO_MD: &str = "---\nname: demo\ndescription: A demo\n---\nBody\n";

    fn make_skill_folder(dir: &Path) {
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("SKILL.md"), DEMO_MD).unwrap();
        std::fs::write(dir.join("scripts/run.sh"), "echo hi\n").unwrap();
        std::fs::set_permissions(dir.join("scripts/run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn skill_refusal(s: &FileSkill) -> Refusal {
        match decode_skill(s) {
            Err(r) => r,
            Ok(_) => panic!("expected a refusal"),
        }
    }

    #[test]
    fn a_skill_folder_reads_as_sorted_entries_without_links_or_ds_store() {
        let dir = tempfile::tempdir().unwrap();
        make_skill_folder(dir.path());
        std::fs::write(dir.path().join(".DS_Store"), "x").unwrap();
        std::os::unix::fs::symlink("SKILL.md", dir.path().join("link.md")).unwrap();
        let blobs = read_skill_folder(dir.path()).unwrap();
        let paths: Vec<&str> = blobs.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, ["SKILL.md", "scripts/run.sh"]);
        assert!(!blobs[0].executable);
        assert!(blobs[1].executable);
    }

    #[test]
    fn a_folder_size_counts_only_what_reading_the_folder_keeps() {
        let dir = tempfile::tempdir().unwrap();
        make_skill_folder(dir.path());
        std::fs::write(dir.path().join(".DS_Store"), "xxxx").unwrap();
        std::os::unix::fs::symlink("SKILL.md", dir.path().join("link.md")).unwrap();
        std::os::unix::fs::symlink("gone.md", dir.path().join("dangling.md")).unwrap();
        std::os::unix::fs::symlink("..", dir.path().join("scripts/up")).unwrap();
        let kept: u64 = read_skill_folder(dir.path()).unwrap().iter().map(|b| b.bytes.len() as u64).sum();
        assert_eq!(folder_size(dir.path()).unwrap(), kept);
        assert_eq!(kept, (DEMO_MD.len() + "echo hi\n".len()) as u64);
    }

    #[test]
    fn an_unreadable_folder_is_named_in_the_error() {
        let dir = tempfile::tempdir().unwrap();
        make_skill_folder(dir.path());
        let locked = dir.path().join("scripts");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let size = folder_size(dir.path());
        let read = read_skill_folder(dir.path());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        for err in [size.unwrap_err(), read.unwrap_err()] {
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
            assert!(err.to_string().contains(&locked.display().to_string()), "{err}");
        }
    }

    #[test]
    fn text_binary_and_modes_round_trip_through_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        make_skill_folder(dir.path());
        std::fs::write(dir.path().join("logo.png"), [0x89, b'P', b'N', b'G', 0xff, 0xfe, 0x00]).unwrap();
        let first = read_skill_folder(dir.path()).unwrap();
        let entries: Vec<FileEntry> = first.iter().map(to_entry).collect();
        let png = entries.iter().find(|e| e.path == "logo.png").unwrap();
        assert!(png.base64.is_some() && png.text.is_none());
        let decoded = decode_skill(&skill("demo", entries)).unwrap();
        assert_eq!(decoded.file.name, "demo");
        let out = tempfile::tempdir().unwrap();
        write_blobs(out.path(), &decoded.blobs).unwrap();
        assert_eq!(read_skill_folder(out.path()).unwrap(), first);
    }

    #[test]
    fn bad_paths_make_the_skill_invalid() {
        for path in ["/etc/x", "a/../b", "a//b", "./a", "a/.", "...", "", "a\0b"] {
            let s = skill("demo", vec![entry(path, "x"), entry("SKILL.md", DEMO_MD)]);
            assert!(matches!(skill_refusal(&s), Refusal::SkillPathInvalid(p) if p == path), "{path:?}");
        }
        let s =
            skill("demo", vec![entry("Scripts/run.sh", "a"), entry("scripts/run.sh", "b"), entry("SKILL.md", DEMO_MD)]);
        assert!(matches!(skill_refusal(&s), Refusal::SkillPathRepeated(p) if p == "scripts/run.sh"));
        let s = skill("demo", vec![entry("scripts", "a"), entry("scripts/run.sh", "b"), entry("SKILL.md", DEMO_MD)]);
        assert!(matches!(skill_refusal(&s), Refusal::SkillPathConflict(p) if p == "scripts"));
        let both =
            FileEntry { path: "a".into(), executable: false, text: Some("x".into()), base64: Some("eA==".into()) };
        let neither = FileEntry { path: "b".into(), executable: false, text: None, base64: None };
        let bad64 = FileEntry { path: "c".into(), executable: false, text: None, base64: Some("!!".into()) };
        for e in [both, neither, bad64] {
            let path = e.path.clone();
            let s = skill("demo", vec![e, entry("SKILL.md", DEMO_MD)]);
            assert!(matches!(skill_refusal(&s), Refusal::SkillFileContent(p) if p == path));
        }
    }

    #[test]
    fn skill_md_must_be_there_parse_and_name_the_slug() {
        let s = skill("demo", vec![entry("other.md", "x")]);
        assert!(matches!(skill_refusal(&s), Refusal::SkillNoFile(p) if p == "demo"));
        let s = skill("demo", vec![entry("SKILL.md", "no frontmatter")]);
        assert!(matches!(skill_refusal(&s), Refusal::SkillBadFrontmatter(_)));
        let s = skill("demo", vec![entry("SKILL.md", "---\nname: other\ndescription: d\n---\nB\n")]);
        assert!(matches!(skill_refusal(&s), Refusal::SkillNameNotSlug(p) if p == "demo"));
        let s = skill("Bad_Slug", vec![entry("SKILL.md", DEMO_MD)]);
        assert!(matches!(skill_refusal(&s), Refusal::SkillSlugInvalid(p) if p == "Bad_Slug"));
    }

    #[test]
    fn a_skill_over_50_mb_is_invalid() {
        let big = "a".repeat(MAX_SKILL_BYTES as usize + 1);
        let s = skill("demo", vec![entry("SKILL.md", DEMO_MD), entry("big.txt", &big)]);
        assert!(matches!(skill_refusal(&s), Refusal::SkillTooLarge { mb: 50 }));
    }
}
