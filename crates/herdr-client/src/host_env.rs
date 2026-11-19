use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

/// AppImage's loader paths belong to the GUI, not to the user's shell or agents.
pub fn host_command_env(command: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR") else { return };
    for name in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
        command.env_remove(name);
    }
    for name in [
        "PATH",
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "XDG_DATA_DIRS",
        "GTK_PATH",
        "GTK_EXE_PREFIX",
        "GTK_DATA_PREFIX",
        "GIO_MODULE_DIR",
        "GIO_EXTRA_MODULES",
        "GSETTINGS_SCHEMA_DIR",
        "GDK_PIXBUF_MODULE_FILE",
        "GDK_PIXBUF_MODULEDIR",
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_SYSTEM_PATH",
    ] {
        if let Some(value) = std::env::var_os(name) {
            match without_bundle(&value, Path::new(&appdir)) {
                Some(value) => {
                    command.env(name, value);
                }
                None => {
                    command.env_remove(name);
                }
            }
        }
    }
}

fn without_bundle(value: &OsStr, appdir: &Path) -> Option<OsString> {
    let paths: Vec<_> =
        std::env::split_paths(value).filter(|p| !p.as_os_str().is_empty() && !p.starts_with(appdir)).collect();
    if paths.is_empty() {
        None
    } else {
        std::env::join_paths(paths).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_the_bundled_paths_and_preserves_host_tools() {
        let app = Path::new("/tmp/Sonata.AppDir");
        assert_eq!(
            without_bundle(OsStr::new("/tmp/Sonata.AppDir/usr/lib:/usr/local/lib:/usr/lib"), app),
            Some(OsString::from("/usr/local/lib:/usr/lib"))
        );
        assert_eq!(without_bundle(OsStr::new("/tmp/Sonata.AppDir/usr/lib/cache"), app), None);
        assert_eq!(
            without_bundle(OsStr::new("/tmp/Sonata.AppDirectory/lib:/bin"), app),
            Some(OsString::from("/tmp/Sonata.AppDirectory/lib:/bin"))
        );
    }
}
