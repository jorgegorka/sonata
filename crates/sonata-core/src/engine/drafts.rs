use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::model::Session;

const DRAFTS_FOLDER: &str = "skill-drafts";

pub(super) fn drafts_relative(session: &Session) -> PathBuf {
    Path::new(DRAFTS_FOLDER).join(format!("session-{}", session.number))
}

pub(super) fn folder_fingerprint(dir: &Path) -> std::io::Result<String> {
    let mut files = Vec::new();
    collect_files(dir, &[], &mut files)?;
    files.sort();
    let mut hash = Sha256::new();
    for (relative, path) in files {
        let bytes = std::fs::read(&path)?;
        hash.update(&relative);
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn collect_files(dir: &Path, prefix: &[u8], files: &mut Vec<(Vec<u8>, PathBuf)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let mut relative = prefix.to_vec();
        if !relative.is_empty() {
            relative.push(b'/');
        }
        relative.extend_from_slice(entry.file_name().as_encoded_bytes());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(&entry.path(), &relative, files)?;
        } else if kind.is_file() {
            files.push((relative, entry.path()));
        }
    }
    Ok(())
}

pub(super) fn draft_links(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    collect_links(dir, &mut links)?;
    links.sort();
    Ok(links)
}

fn collect_links(dir: &Path, links: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            links.push(path);
        } else if meta.is_dir() {
            collect_links(&path, links)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(parent: &Path, name: &str, skill_first: bool) -> PathBuf {
        let dir = parent.join(name);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        let skill = || std::fs::write(dir.join("SKILL.md"), "hello").unwrap();
        let script = || std::fs::write(dir.join("scripts/run.sh"), "hi\n").unwrap();
        if skill_first {
            skill();
            script();
        } else {
            script();
            skill();
        }
        dir
    }

    #[test]
    fn a_fingerprint_hashes_each_files_path_length_and_bytes_in_path_order() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert_eq!(
            folder_fingerprint(&empty).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let one = draft(tmp.path(), "one", true);
        assert_eq!(
            folder_fingerprint(&one).unwrap(),
            "65e133e5791e3bb42639e759a56cdac450e813148966da692b0d49b8fec058c7"
        );
        assert_eq!(folder_fingerprint(&draft(tmp.path(), "two", false)).unwrap(), folder_fingerprint(&one).unwrap());
        std::fs::create_dir(one.join("notes")).unwrap();
        let before = folder_fingerprint(&one).unwrap();
        assert_eq!(before, "65e133e5791e3bb42639e759a56cdac450e813148966da692b0d49b8fec058c7");
        std::fs::write(one.join("SKILL.md"), "hellO").unwrap();
        assert_ne!(folder_fingerprint(&one).unwrap(), before);
        std::fs::write(one.join("SKILL.md"), "hello").unwrap();
        std::fs::rename(one.join("scripts/run.sh"), one.join("scripts/go.sh")).unwrap();
        assert_ne!(folder_fingerprint(&one).unwrap(), before);
        std::fs::rename(one.join("scripts/go.sh"), one.join("notes/run.sh")).unwrap();
        assert_ne!(folder_fingerprint(&one).unwrap(), before);
    }

    #[test]
    fn links_are_found_at_any_depth_and_never_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = draft(tmp.path(), "seo", true);
        assert!(draft_links(&dir).unwrap().is_empty());
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(outside.join("deep")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("scripts/keys")).unwrap();
        std::os::unix::fs::symlink(dir.join("SKILL.md"), dir.join("COPY.md")).unwrap();
        assert_eq!(draft_links(&dir).unwrap(), vec![dir.join("COPY.md"), dir.join("scripts/keys")]);
    }
}
