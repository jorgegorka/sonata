use std::path::{Path, PathBuf};

pub use herdr_client::host_command_env as host_command;

/// Linux packages keep helpers out of /usr/bin to avoid owning another app's herdr.
pub fn runtime_dir(executable: &Path, resources: Option<&Path>) -> PathBuf {
    if cfg!(target_os = "linux") {
        if let Some(dir) = resources.map(|p| p.join("runtime")).filter(|p| p.is_dir()) {
            return dir;
        }
    }
    executable.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_is_private_in_linux_packages_and_adjacent_in_development() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("bin/sonata");
        let resources = tmp.path().join("lib/sonata");
        assert_eq!(runtime_dir(&exe, None), tmp.path().join("bin"));
        std::fs::create_dir_all(resources.join("runtime")).unwrap();
        let expected = if cfg!(target_os = "linux") { resources.join("runtime") } else { tmp.path().join("bin") };
        assert_eq!(runtime_dir(&exe, Some(&resources)), expected);
    }
}
