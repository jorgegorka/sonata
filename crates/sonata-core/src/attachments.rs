use std::path::{Path, PathBuf};

use crate::model::Attachment;

pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AttachmentError {
    #[error("{0} does not exist")]
    NotFound(String),
    #[error("{0} is a folder; attach the files in it one by one")]
    NotAFile(String),
    #[error("{path} is {mb} MB; the limit is 100 MB per file")]
    TooLarge { path: String, mb: u64 },
    #[error("could not copy {path}: {error}")]
    Copy { path: String, error: String },
}

pub fn check(paths: &[String], base: &Path) -> Result<Vec<PathBuf>, AttachmentError> {
    paths
        .iter()
        .map(|p| {
            let path = base.join(p);
            let shown = path.to_string_lossy().into_owned();
            let meta = std::fs::metadata(&path).map_err(|_| AttachmentError::NotFound(shown.clone()))?;
            if !meta.is_file() {
                return Err(AttachmentError::NotAFile(shown));
            }
            if meta.len() > MAX_FILE_BYTES {
                return Err(AttachmentError::TooLarge { path: shown, mb: meta.len().div_ceil(1024 * 1024) });
            }
            Ok(path)
        })
        .collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "file".into())
}

pub fn copy_to_inbox(files: &[PathBuf], bot_dir: &Path, message_id: &str) -> Result<Vec<Attachment>, AttachmentError> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let inbox = bot_dir.join("inbox");
    std::fs::create_dir_all(&inbox)
        .map_err(|e| AttachmentError::Copy { path: inbox.to_string_lossy().into_owned(), error: e.to_string() })?;
    let mut copied: Vec<Attachment> = Vec::new();
    for file in files {
        let name = file_name(file);
        match copy_new(file, &inbox, message_id, &name) {
            Ok(dest) => copied.push(Attachment { name, path: dest.to_string_lossy().into_owned() }),
            Err(e) => {
                for done in &copied {
                    let _ = std::fs::remove_file(&done.path);
                }
                return Err(AttachmentError::Copy { path: file.to_string_lossy().into_owned(), error: e.to_string() });
            }
        }
    }
    Ok(copied)
}

fn copy_new(file: &Path, inbox: &Path, message_id: &str, name: &str) -> std::io::Result<PathBuf> {
    let mut src = std::fs::File::open(file)?;
    let mut n = 1;
    loop {
        let stored = if n == 1 { format!("{message_id}-{name}") } else { format!("{message_id}-{n}-{name}") };
        let dest = inbox.join(stored);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(mut out) => {
                if let Err(e) = std::io::copy(&mut src, &mut out) {
                    let _ = std::fs::remove_file(&dest);
                    return Err(e);
                }
                return Ok(dest);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

pub fn in_place(files: &[PathBuf]) -> Vec<Attachment> {
    files.iter().map(|f| Attachment { name: file_name(f), path: f.to_string_lossy().into_owned() }).collect()
}

pub const RUNNABLE_EXTENSIONS: &[&str] = &[
    "app",
    "applescript",
    "bash",
    "command",
    "csh",
    "fileloc",
    "fish",
    "inetloc",
    "jar",
    "ksh",
    "mobileconfig",
    "mpkg",
    "pkg",
    "pl",
    "py",
    "rb",
    "scpt",
    "sh",
    "terminal",
    "tool",
    "webloc",
    "workflow",
    "zsh",
];

pub fn runs_programs(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let listed = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RUNNABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
    listed || std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn check_refuses_missing_files_folders_and_files_over_100_mb() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = write(tmp.path(), "a.md", "a");
        assert_eq!(check(&[s(&ok)], Path::new("/")).unwrap(), vec![ok.clone()]);
        let missing = s(&tmp.path().join("nope.md"));
        assert_eq!(check(&[s(&ok), missing.clone()], Path::new("/")), Err(AttachmentError::NotFound(missing)));
        assert_eq!(check(&[s(tmp.path())], Path::new("/")), Err(AttachmentError::NotAFile(s(tmp.path()))));
        let big = tmp.path().join("big.bin");
        std::fs::File::create(&big).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        assert_eq!(check(&[s(&big)], Path::new("/")), Err(AttachmentError::TooLarge { path: s(&big), mb: 101 }));
    }

    #[test]
    fn relative_paths_resolve_against_the_base() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "draft.md", "d");
        assert_eq!(check(&["draft.md".into()], tmp.path()).unwrap(), vec![tmp.path().join("draft.md")]);
    }

    #[test]
    fn copies_keep_their_names_inside_the_inbox_and_same_names_are_both_kept() {
        let src = tempfile::tempdir().unwrap();
        let a = write(src.path(), "notas de reunión.md", "first");
        let b = write(&src.path().join("sub"), "notas de reunión.md", "second");
        let bot = tempfile::tempdir().unwrap();
        let copied = copy_to_inbox(&[a, b], bot.path(), "01JMSG").unwrap();
        let first = bot.path().join("inbox/01JMSG-notas de reunión.md");
        let second = bot.path().join("inbox/01JMSG-2-notas de reunión.md");
        assert_eq!(
            copied,
            vec![
                Attachment { name: "notas de reunión.md".into(), path: s(&first) },
                Attachment { name: "notas de reunión.md".into(), path: s(&second) },
            ]
        );
        assert_eq!(std::fs::read_to_string(first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "second");
    }

    #[test]
    fn names_that_differ_only_in_case_or_unicode_form_are_both_kept() {
        let src = tempfile::tempdir().unwrap();
        let upper = write(src.path(), "Notes.md", "upper");
        let lower = write(&src.path().join("sub"), "notes.md", "lower");
        let nfc = write(&src.path().join("nfc"), "reuni\u{f3}n.md", "nfc");
        let nfd = write(&src.path().join("nfd"), "reunio\u{301}n.md", "nfd");
        let bot = tempfile::tempdir().unwrap();
        let copied = copy_to_inbox(&[upper, lower, nfc, nfd], bot.path(), "01JMSG").unwrap();
        let texts: Vec<String> = copied.iter().map(|a| std::fs::read_to_string(&a.path).unwrap()).collect();
        assert_eq!(texts, ["upper", "lower", "nfc", "nfd"], "no copy overwrote another: {copied:?}");
    }

    #[test]
    fn a_link_in_the_inbox_is_never_written_through() {
        let src = tempfile::tempdir().unwrap();
        let file = write(src.path(), "a.md", "new");
        let bot = tempfile::tempdir().unwrap();
        let target = write(bot.path(), "precious.txt", "keep");
        std::fs::create_dir_all(bot.path().join("inbox")).unwrap();
        std::os::unix::fs::symlink(&target, bot.path().join("inbox/01JMSG-a.md")).unwrap();
        let copied = copy_to_inbox(&[file], bot.path(), "01JMSG").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        assert!(copied[0].path.ends_with("inbox/01JMSG-2-a.md"), "{copied:?}");
    }

    #[test]
    fn scripts_installers_and_executables_count_as_programs() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let note = write(tmp.path(), "report.md", "r");
        assert!(!runs_programs(&note));
        assert!(runs_programs(&write(tmp.path(), "setup.command", "echo")));
        assert!(runs_programs(&write(tmp.path(), "Installer.PKG", "x")));
        assert!(runs_programs(&write(tmp.path(), "site.webloc", "x")), "opens whatever it points to");
        assert!(runs_programs(&write(tmp.path(), "server.inetloc", "x")));
        std::fs::set_permissions(&note, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(runs_programs(&note), "an executable bit counts too");
    }

    #[test]
    fn files_for_the_user_stay_where_they_are() {
        assert_eq!(
            in_place(&[PathBuf::from("/Users/x/Bots/writer/report.md")]),
            vec![Attachment { name: "report.md".into(), path: "/Users/x/Bots/writer/report.md".into() }]
        );
    }
}
