use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub const HERDR_SESSION: &str = "sonata";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub bots_root: PathBuf,
}

impl AppPaths {
    pub fn new(data_dir: impl Into<PathBuf>, bots_root: impl Into<PathBuf>) -> Self {
        Self { data_dir: data_dir.into(), bots_root: bots_root.into() }
    }

    pub fn from_env_or_default() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let data = std::env::var_os("SONATA_DATA_DIR").map(PathBuf::from).unwrap_or_else(default_data_dir);
        let bots = std::env::var_os("SONATA_BOTS_ROOT").map(PathBuf::from).unwrap_or_else(|| home.join("Sonata/Bots"));
        Self::new(data, bots)
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("sonata.db")
    }

    pub fn bridge_socket(&self) -> PathBuf {
        self.data_dir.join("bridge.sock")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }

    pub fn previous_dir(&self) -> PathBuf {
        self.data_dir.join("previous")
    }

    pub fn bot_dir(&self, handle: &str) -> PathBuf {
        self.bots_root.join(handle)
    }

    pub fn ensure_data(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.logs_dir())?;
        std::fs::create_dir_all(self.skills_dir())
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.logs_dir())?;
        std::fs::create_dir_all(self.skills_dir())?;
        std::fs::create_dir_all(&self.bots_root)
    }

    pub fn lock(&self) -> io::Result<File> {
        let path = self.data_dir.join("app.lock");
        let file = OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("another Sonata instance is already running with {}", self.data_dir.display()),
            )),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }

    pub fn herdr_session(&self) -> String {
        herdr_session_name(&self.data_dir, &default_data_dir())
    }
}

pub fn resolve_bots_root(env_override: Option<PathBuf>, saved: Option<String>, default: PathBuf) -> PathBuf {
    env_override.or_else(|| saved.map(PathBuf::from)).unwrap_or(default)
}

pub fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    return dirs::home_dir().unwrap_or_default().join("Library/Application Support/Sonata");
    #[cfg(not(target_os = "macos"))]
    return dirs::data_dir().unwrap_or_default().join("sonata");
}

pub fn herdr_session_name(data_dir: &Path, default_data_dir: &Path) -> String {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let data_dir = canonical(data_dir);
    if data_dir == canonical(default_data_dir) {
        return HERDR_SESSION.to_string();
    }
    format!("{HERDR_SESSION}-{:08x}", fnv1a32(data_dir.as_os_str().as_bytes()))
}

fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, b| (hash ^ u32::from(*b)).wrapping_mul(0x0100_0193))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_ensure() {
        let tmp = tempfile::tempdir().unwrap();
        let p = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
        p.ensure().unwrap();
        assert!(p.logs_dir().is_dir());
        assert!(p.bots_root.is_dir());
        let q = AppPaths::new(tmp.path().join("data2"), tmp.path().join("bots2"));
        q.ensure_data().unwrap();
        assert!(q.logs_dir().is_dir() && !q.bots_root.exists());
        assert_eq!(p.bot_dir("researcher"), tmp.path().join("bots/researcher"));
        assert_eq!(p.bridge_socket(), tmp.path().join("data/bridge.sock"));
    }

    #[test]
    fn the_skill_library_lives_in_the_data_dir_and_is_created_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
        assert_eq!(paths.skills_dir(), tmp.path().join("data/skills"));
        paths.ensure_data().unwrap();
        assert!(paths.skills_dir().is_dir());
        assert_eq!(paths.previous_dir(), tmp.path().join("data/previous"));
    }

    #[test]
    fn the_bots_folder_is_the_env_override_then_the_saved_one_then_the_default() {
        let default = || PathBuf::from("/default");
        let saved = || Some("/saved".to_string());
        assert_eq!(resolve_bots_root(Some("/env".into()), saved(), default()), PathBuf::from("/env"));
        assert_eq!(resolve_bots_root(None, saved(), default()), PathBuf::from("/saved"));
        assert_eq!(resolve_bots_root(None, None, default()), PathBuf::from("/default"));
    }

    #[test]
    fn herdr_session_is_sonata_agents_for_the_default_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let default = tmp.path().join("Sonata");
        std::fs::create_dir_all(&default).unwrap();
        assert_eq!(herdr_session_name(&default, &default), "sonata");
        assert_eq!(herdr_session_name(&default.join("../Sonata"), &default), "sonata");
    }

    #[test]
    fn herdr_session_for_another_data_dir_is_a_stable_hash_of_its_path() {
        let default = Path::new("/nonexistent/default");
        assert_eq!(herdr_session_name(Path::new("/nonexistent/sonata-data-a"), default), "sonata-e6de9a7b");
        assert_eq!(herdr_session_name(Path::new("/nonexistent/sonata-data-b"), default), "sonata-e7de9c0e");
        assert_eq!(fnv1a32(b"a"), 0xe40c292c);

        let tmp = tempfile::tempdir().unwrap();
        let name = herdr_session_name(tmp.path(), default);
        assert_eq!(name, herdr_session_name(tmp.path(), default));
        let hex = name.strip_prefix("sonata-").unwrap();
        assert!(hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn app_lock_admits_one_instance_per_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let p = AppPaths::new(tmp.path().join("data"), tmp.path().join("bots"));
        p.ensure().unwrap();
        let first = p.lock().unwrap();
        let err = p.lock().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert!(err.to_string().contains("already running"), "{err}");
        drop(first);
        assert!(p.lock().is_ok(), "the lock is released with its file");
    }

    #[test]
    fn default_bridge_socket_fits_macos_sun_path_limit() {
        let home = dirs::home_dir().unwrap();
        let p = AppPaths::new(home.join("Library/Application Support/Sonata"), home.join("x"));
        assert!(p.bridge_socket().as_os_str().len() < 104);
    }
}
