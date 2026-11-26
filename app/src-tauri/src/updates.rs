// Application-owned update state. Downloaded bytes and destinations never come from IPC.
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

use reqwest::Url;
use semver::Version;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use sonata_core::engine::Engine;
use tauri::{Emitter, Manager, State, Webview};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::CommandError;

const FEED_ROOT: &str = "https://jorgegorka.github.io/sonata/updates";
const MAX_METADATA: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Checking,
    Available,
    Downloading,
    Ready,
    Installing,
    DownloadFailed,
    InstallFailed,
    Dismissed,
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Installation {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Macos,
    Appimage,
    Deb,
    Arch,
    Extracted,
}

impl Installation {
    fn native(self) -> bool {
        matches!(self, Self::Macos | Self::Appimage | Self::Deb)
    }
    fn target(self, arch: &str) -> String {
        match self {
            Self::Macos => format!("darwin-{arch}"),
            Self::Appimage | Self::Extracted => format!("linux-{arch}-appimage"),
            Self::Deb => format!("linux-{arch}-deb"),
            Self::Arch => format!("linux-{arch}-arch"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateState {
    pub revision: u64,
    pub phase: Phase,
    pub version: Option<String>,
    pub installation: Option<Installation>,
    pub downloaded: usize,
    pub total: Option<u64>,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self { revision: 0, phase: Phase::Idle, version: None, installation: None, downloaded: 0, total: None }
    }
}

#[derive(Debug, Deserialize)]
struct Feed {
    version: String,
    url: String,
    notes_url: String,
    #[serde(default)]
    signature: String,
}

#[derive(Default)]
struct Inner {
    view: UpdateState,
    update: Option<Update>,
    feed: Option<Feed>,
    bytes: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct Updates {
    checked: AtomicBool,
    operation: tokio::sync::Mutex<()>,
    inner: Mutex<Inner>,
}

impl Updates {
    pub fn prevents_exit(&self, code: Option<i32>) -> bool {
        self.snapshot().phase == Phase::Installing && code != Some(tauri::RESTART_EXIT_CODE)
    }

    fn snapshot(&self) -> UpdateState {
        self.inner.lock().unwrap().view.clone()
    }
    fn change(&self, app: &tauri::AppHandle, change: impl FnOnce(&mut Inner)) -> UpdateState {
        let state = {
            let mut inner = self.inner.lock().unwrap();
            change(&mut inner);
            inner.view.revision += 1;
            inner.view.clone()
        };
        let _ = app.emit_to("main", "update-state", &state);
        state
    }
}

fn refusal(message: &str) -> CommandError {
    CommandError::internal(message)
}
fn require_main(label: &str) -> Result<(), CommandError> {
    if label == "main" {
        Ok(())
    } else {
        Err(refusal("Updates are only available in the main window"))
    }
}

fn newer_stable(version: &str, current: &Version) -> bool {
    Version::parse(version).is_ok_and(|v| v.pre.is_empty() && v.build.is_empty() && v > *current)
}

fn trusted_url(value: &str, download: bool) -> bool {
    let Ok(url) = Url::parse(value) else { return false };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    match url.host_str() {
        Some("github.com") => {
            let prefix = if download { "/jorgegorka/sonata/releases/download/" } else { "/jorgegorka/sonata/" };
            url.path().starts_with(prefix) && !url.path().contains('%')
        }
        Some("jorgegorka.github.io") if download => {
            url.path().starts_with("/sonata/downloads/") && !url.path().contains('%')
        }
        _ => false,
    }
}

fn validate_feed(
    raw: &serde_json::Value,
    current: &Version,
    installation: Installation,
) -> Result<Option<Feed>, String> {
    let feed: Feed = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
    if !newer_stable(&feed.version, current) {
        return Ok(None);
    }
    if !trusted_url(&feed.url, true) || !trusted_url(&feed.notes_url, false) {
        return Err("Untrusted update URL".into());
    }
    if installation.native() && feed.signature.trim().is_empty() {
        return Err("The update has no signature".into());
    }
    Ok(Some(feed))
}

// Arch packages are repacked DEBs, so package markers take precedence over Tauri's bundle type.
// Extracted AppImages have no APPIMAGE; never replace their usr/bin/sonata with an AppImage.
#[cfg(any(target_os = "linux", test))]
fn linux_installation(
    exe: &Path,
    appimage: Option<&Path>,
    arch: bool,
    extracted: bool,
    deb: bool,
) -> Option<Installation> {
    if appimage.is_some_and(|p| p.is_file()) {
        Some(Installation::Appimage)
    } else if arch && exe == Path::new("/usr/bin/sonata") {
        Some(Installation::Arch)
    } else if extracted {
        Some(Installation::Extracted)
    } else if deb && exe == Path::new("/usr/bin/sonata") {
        Some(Installation::Deb)
    } else {
        None
    }
}

fn installation(app: &tauri::AppHandle) -> Option<Installation> {
    if cfg!(debug_assertions) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        let root = exe.parent()?.parent()?.parent()?;
        (root.extension().is_some_and(|e| e == "app") && root.join("Contents/Info.plist").is_file())
            .then_some(Installation::Macos)
    }
    #[cfg(target_os = "linux")]
    {
        let appimage = app.env().appimage;
        let arch = Path::new("/usr/share/sonata/install-format").is_file()
            || (Path::new("/usr/bin/pacman").is_file()
                && std::process::Command::new("/usr/bin/pacman")
                    .args(["-Qoq", "/usr/bin/sonata"])
                    .output()
                    .is_ok_and(|o| o.status.success() && o.stdout == b"sonata\n"));
        let extracted = exe.ancestors().take(5).any(|p| p.join(".sonata-version").is_file());
        let deb = tauri::utils::platform::bundle_type() == Some(tauri::utils::config::BundleType::Deb)
            && std::process::Command::new("/usr/bin/dpkg-query").args(["-S", "/usr/bin/sonata"]).output().is_ok_and(
                |o| {
                    o.status.success()
                        && String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .any(|l| l == "sonata: /usr/bin/sonata" || l == "sonata-app: /usr/bin/sonata")
                },
            );
        linux_installation(&exe, appimage.as_deref().map(Path::new), arch, extracted, deb)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (exe, app);
        None
    }
}

async fn fetch_feed(url: &str) -> Result<Option<serde_json::Value>, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("Sonata/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client.get(url).send().await.map_err(|e| e.to_string())?;
    if response.status() == 404 || response.status() == 204 {
        return Ok(None);
    }
    response.error_for_status_ref().map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > MAX_METADATA {
            return Err("Update metadata is too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map(Some).map_err(|e| e.to_string())
}

fn restore_native_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    raw: serde_json::Value,
    target: &str,
) -> Result<Update, String> {
    let endpoint = Url::parse(&format!("{FEED_ROOT}/{target}.json")).map_err(|e| e.to_string())?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .configure_client(|client| client.https_only(true))
        .build()
        .map_err(|e| e.to_string())?;
    let mut update = updater
        .restore_update(raw)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Update version was not newer".to_string())?;
    update.timeout = Some(Duration::from_secs(300));
    Ok(update)
}

#[tauri::command]
pub fn get_update_state(webview: Webview, updates: State<'_, Updates>) -> Result<UpdateState, CommandError> {
    require_main(webview.label())?;
    Ok(updates.snapshot())
}

#[tauri::command]
pub fn set_automatic_updates(
    webview: Webview,
    app: tauri::AppHandle,
    engine: State<'_, Engine>,
    updates: State<'_, Updates>,
    enabled: bool,
) -> Result<(), CommandError> {
    require_main(webview.label())?;
    engine.set_automatic_updates(enabled)?;
    if !enabled {
        updates.change(&app, |s| {
            if matches!(s.view.phase, Phase::Checking | Phase::Available) {
                s.view.phase = Phase::Dismissed;
                s.update = None;
                s.feed = None;
            }
        });
    }
    Ok(())
}

#[tauri::command]
pub async fn check_startup_update(
    webview: Webview,
    app: tauri::AppHandle,
    engine: State<'_, Engine>,
    updates: State<'_, Updates>,
) -> Result<UpdateState, CommandError> {
    require_main(webview.label())?;
    if updates.checked.swap(true, Ordering::SeqCst) {
        return Ok(updates.snapshot());
    }
    if !engine.automatic_updates()? {
        return Ok(updates.snapshot());
    }
    // Keep any blocking package-manager detection off the UI/runtime worker.
    let handle = app.clone();
    let format =
        tauri::async_runtime::spawn_blocking(move || installation(&handle)).await.map_err(CommandError::internal)?;
    let Some(format) = format else {
        return Ok(updates.snapshot());
    };
    let _operation = updates.operation.lock().await;
    if !engine.automatic_updates()? {
        return Ok(updates.snapshot());
    }
    updates.change(&app, |s| {
        s.view.phase = Phase::Checking;
    });
    let result = async {
        let target = format.target(std::env::consts::ARCH);
        let Some(raw) = fetch_feed(&format!("{FEED_ROOT}/{target}.json")).await? else {
            return Ok(None);
        };
        let Some(feed) = validate_feed(&raw, &app.package_info().version, format)? else {
            return Ok(None);
        };
        let update = if format.native() { Some(restore_native_update(&app, raw, &target)?) } else { None };
        Ok::<_, String>(Some((feed, update)))
    }
    .await;
    let enabled = engine.automatic_updates()?;
    Ok(updates.change(&app, |s| {
        if s.view.phase == Phase::Dismissed || !enabled {
            return;
        }
        match result {
            Ok(Some((feed, update))) => {
                s.view.phase = Phase::Available;
                s.view.version = Some(feed.version.clone());
                s.view.installation = Some(format);
                s.feed = Some(feed);
                s.update = update;
            }
            Ok(None) => s.view.phase = Phase::Idle,
            Err(error) => {
                tracing::warn!(%error, "startup update check failed");
                s.view.phase = Phase::Idle;
            }
        }
    }))
}

#[tauri::command]
pub async fn download_update(
    webview: Webview,
    app: tauri::AppHandle,
    updates: State<'_, Updates>,
) -> Result<UpdateState, CommandError> {
    require_main(webview.label())?;
    let _operation = updates.operation.try_lock().map_err(|_| refusal("An update operation is already running"))?;
    let update = {
        let s = updates.inner.lock().unwrap();
        if !matches!(s.view.phase, Phase::Available | Phase::DownloadFailed) {
            return Err(refusal("No update is ready to download"));
        }
        s.update.clone().ok_or_else(|| refusal("This installation needs a guided update"))?
    };
    updates.change(&app, |s| {
        s.view.phase = Phase::Downloading;
        s.view.downloaded = 0;
        s.view.total = None;
    });
    let mut downloaded = 0;
    let mut progress_at = Instant::now();
    let result = update
        .download(
            |chunk, total| {
                downloaded += chunk;
                if progress_at.elapsed() >= Duration::from_millis(100) || total == Some(downloaded as u64) {
                    updates.change(&app, |s| {
                        s.view.downloaded = downloaded;
                        s.view.total = total;
                    });
                    progress_at = Instant::now();
                }
            },
            || {},
        )
        .await;
    Ok(updates.change(&app, |s| match result {
        Ok(bytes) => {
            s.view.downloaded = bytes.len();
            s.bytes = Some(bytes);
            s.view.phase = Phase::Ready;
        }
        Err(error) => {
            tracing::warn!(%error, "update download or verification failed");
            s.view.phase = Phase::DownloadFailed;
        }
    }))
}

#[tauri::command]
pub async fn install_update(
    webview: Webview,
    app: tauri::AppHandle,
    engine: State<'_, Engine>,
    updates: State<'_, Updates>,
) -> Result<UpdateState, CommandError> {
    require_main(webview.label())?;
    let _operation = updates.operation.try_lock().map_err(|_| refusal("An update operation is already running"))?;
    let (update, bytes) = {
        let mut s = updates.inner.lock().unwrap();
        if s.view.phase != Phase::Ready {
            return Err(refusal("No verified update is ready"));
        }
        (
            s.update.clone().ok_or_else(|| refusal("No pending update"))?,
            s.bytes.take().ok_or_else(|| refusal("No downloaded update"))?,
        )
    };
    // Verify again before stopping work; installation also verifies before changing files.
    if let Err(error) = update.verify(&bytes) {
        tracing::warn!(%error, "downloaded update failed re-verification");
        return Ok(updates.change(&app, |s| s.view.phase = Phase::DownloadFailed));
    }
    updates.change(&app, |s| s.view.phase = Phase::Installing);
    if let Err(error) = engine.shutdown_for_update().await {
        tracing::error!(%error, "runtime could not stop; refusing to replace the application");
        return Ok(updates.change(&app, |s| s.view.phase = Phase::InstallFailed));
    }
    let result = tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await;
    match result {
        Ok(Ok(())) => {
            app.request_restart();
            Ok(updates.snapshot())
        }
        error => {
            tracing::error!(?error, "update installation failed after runtime shutdown");
            Ok(updates.change(&app, |s| s.view.phase = Phase::InstallFailed))
        }
    }
}

#[tauri::command]
pub fn restart_after_update_failure(
    webview: Webview,
    app: tauri::AppHandle,
    updates: State<'_, Updates>,
) -> Result<(), CommandError> {
    require_main(webview.label())?;
    if updates.snapshot().phase != Phase::InstallFailed {
        return Err(refusal("No failed installation to recover"));
    }
    app.request_restart();
    Ok(())
}

#[tauri::command]
pub fn dismiss_update(
    webview: Webview,
    app: tauri::AppHandle,
    updates: State<'_, Updates>,
) -> Result<UpdateState, CommandError> {
    require_main(webview.label())?;
    let _operation = updates.operation.try_lock().map_err(|_| refusal("An update operation is already running"))?;
    if matches!(updates.snapshot().phase, Phase::Installing | Phase::InstallFailed) {
        return Err(refusal("Sonata must restart before continuing"));
    }
    Ok(updates.change(&app, |s| {
        s.view.phase = Phase::Dismissed;
        s.bytes = None;
        s.update = None;
        s.feed = None;
    }))
}

#[tauri::command]
pub fn open_update_link(
    webview: Webview,
    app: tauri::AppHandle,
    updates: State<'_, Updates>,
    download: bool,
) -> Result<(), CommandError> {
    require_main(webview.label())?;
    let s = updates.inner.lock().unwrap();
    let feed = s.feed.as_ref().ok_or_else(|| refusal("No available update"))?;
    if download && s.view.installation.is_some_and(Installation::native) {
        return Err(refusal("Use the verified in-app download"));
    }
    let url = if download { &feed.url } else { &feed.notes_url };
    app.opener().open_url(url, None::<String>).map_err(CommandError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_newer_stable_versions() {
        let current = Version::parse("1.9.0").unwrap();
        for value in ["1.9.0", "1.8.99", "2.0.0-beta.1", "2.0.0+test", "latest", ""] {
            assert!(!newer_stable(value, &current), "{value}");
        }
        assert!(newer_stable("1.10.0", &current));
    }

    #[test]
    fn distinguishes_repacked_and_extracted_linux_installs() {
        let exe = Path::new("/usr/bin/sonata");
        assert_eq!(linux_installation(exe, None, true, false, true), Some(Installation::Arch));
        assert_eq!(
            linux_installation(Path::new("/home/user/sonata/usr/bin/sonata"), None, false, true, true),
            Some(Installation::Extracted)
        );
        assert_eq!(linux_installation(exe, None, false, false, true), Some(Installation::Deb));
        assert_eq!(linux_installation(Path::new("/tmp/sonata"), None, false, false, true), None);
        let image = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(linux_installation(exe, Some(image.path()), true, true, true), Some(Installation::Appimage));
        assert_eq!(Installation::Extracted.target("aarch64"), "linux-aarch64-appimage");
    }

    #[test]
    fn requires_trusted_urls_and_signatures_for_native_updates() {
        let current = Version::parse("1.0.0").unwrap();
        let mut raw = json!({"version":"1.1.0","url":"https://github.com/jorgegorka/sonata/releases/download/v1.1.0/app.deb",
            "notes_url":"https://github.com/jorgegorka/sonata/releases/tag/v1.1.0"});
        assert!(validate_feed(&raw, &current, Installation::Deb).is_err());
        assert!(validate_feed(&raw, &current, Installation::Arch).unwrap().is_some());
        raw["signature"] = "signature".into();
        assert!(validate_feed(&raw, &current, Installation::Deb).unwrap().is_some());
        for url in [
            "http://github.com/jorgegorka/sonata/releases/download/v1/a",
            "https://github.com/another/repo/releases/download/v1/a",
            "https://github.com/jorgegorka/sonata/releases/download/../../else",
            "https://jorgegorka.github.io/sonata/downloads/%2e%2e/a",
        ] {
            raw["url"] = url.into();
            assert!(validate_feed(&raw, &current, Installation::Deb).is_err(), "{url}");
        }
    }

    #[test]
    fn startup_is_claimed_once_and_only_main_can_operate_updates() {
        let updates = Updates::default();
        assert!(!updates.checked.swap(true, Ordering::SeqCst));
        assert!(updates.checked.swap(true, Ordering::SeqCst));
        assert!(require_main("main").is_ok());
        for label in ["browser", "pickbox", "remote"] {
            assert!(require_main(label).is_err());
        }
    }
    #[test]
    fn native_updater_uses_the_app_configuration_and_restores_a_feed_without_fetching_again() {
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        *context.config_mut() = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let app =
            tauri::test::mock_builder().plugin(tauri_plugin_updater::Builder::new().build()).build(context).unwrap();
        let raw = json!({
            "version": "99.0.0",
            "url": "https://github.com/jorgegorka/sonata/releases/download/v99.0.0/Sonata-macos-aarch64.app.tar.gz",
            "signature": "invalid-signature",
            "notes_url": "https://github.com/jorgegorka/sonata/releases/tag/v99.0.0"
        });
        let update = restore_native_update(app.handle(), raw, "darwin-aarch64").unwrap();
        assert_eq!(update.version, "99.0.0");
        assert_eq!(update.timeout, Some(Duration::from_secs(300)));
        assert!(update.verify(b"unsigned package").is_err());
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn signed_native_payload_replaces_only_the_disposable_app_and_rejects_tampering() {
        use std::process::{Command, Stdio};
        let temp = tempfile::tempdir().unwrap();
        // A complete ELF header lets the Linux installer recognize the disposable AppImage.
        let executable = |label: &str| {
            let mut bytes = format!("\u{7f}ELF {label}").into_bytes();
            bytes.resize(64, 0);
            bytes
        };
        let old = executable("old executable");
        let new = executable("new executable");
        #[cfg(target_os = "macos")]
        let (installed_binary, archive) = {
            let package = temp.path().join("new/Sonata.app");
            let installed = temp.path().join("installed/Sonata.app");
            for (path, contents) in [(&installed, &old), (&package, &new)] {
                std::fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
                std::fs::write(path.join("Contents/MacOS/sonata"), contents).unwrap();
            }
            let archive = temp.path().join("Sonata.app.tar.gz");
            assert!(Command::new("python3").args(["-c",
                "import sys,tarfile; a=tarfile.open(sys.argv[2], 'w:gz'); a.add(sys.argv[1], arcname='Sonata.app'); a.close()"])
                .arg(&package).arg(&archive).status().unwrap().success());
            (installed.join("Contents/MacOS/sonata"), archive)
        };
        #[cfg(target_os = "linux")]
        let (installed_binary, archive) = {
            use std::os::unix::fs::PermissionsExt;
            let installed = temp.path().join("installed.AppImage");
            let archive = temp.path().join("new.AppImage");
            std::fs::write(&installed, &old).unwrap();
            std::fs::write(&archive, &new).unwrap();
            std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();
            (installed, archive)
        };
        let saved_data = temp.path().join("user-data");
        std::fs::write(&saved_data, "saved projects and settings").unwrap();

        let key = temp.path().join("test.key");
        let app_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let cli = |args: &[&str]| {
            let mut command = Command::new("npm");
            command
                .current_dir(app_dir)
                .args(["exec", "--", "tauri", "signer"])
                .args(args)
                .env_remove("TAURI_SIGNING_PRIVATE_KEY")
                .env_remove("TAURI_SIGNING_PRIVATE_KEY_PATH")
                .env_remove("TAURI_SIGNING_PRIVATE_KEY_PASSWORD")
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        };
        assert!(cli(&["generate", "--ci", "--password", "", "--write-keys"]).arg(&key).status().unwrap().success());
        assert!(cli(&["sign", "--password", "", "--app-version", "99.0.0", "--private-key-path"])
            .arg(&key)
            .arg(&archive)
            .status()
            .unwrap()
            .success());
        let public = std::fs::read_to_string(key.with_extension("key.pub")).unwrap();
        let signature = std::fs::read_to_string(format!("{}.sig", archive.display())).unwrap();
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        *context.config_mut() = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().pubkey(public).build())
            .build(context)
            .unwrap();
        let updater = app
            .updater_builder()
            .endpoints(vec![Url::parse(FEED_ROOT).unwrap()])
            .unwrap()
            .executable_path(&installed_binary)
            .build()
            .unwrap();
        let update = updater
            .restore_update(json!({
                "version": "99.0.0", "url": "https://example.com/unused",
                "signature": signature.trim(),
            }))
            .unwrap()
            .unwrap();
        let bytes = std::fs::read(&archive).unwrap();
        let mut corrupted = bytes.clone();
        corrupted[0] ^= 1;
        assert!(update.install(corrupted).is_err());
        assert_eq!(std::fs::read(&installed_binary).unwrap(), old);
        update.install(bytes).unwrap();
        assert_eq!(std::fs::read(&installed_binary).unwrap(), new);
        assert_eq!(std::fs::read_to_string(saved_data).unwrap(), "saved projects and settings");
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(installed_binary).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }
    #[test]
    fn quitting_cannot_interrupt_installation_but_the_successful_restart_can_proceed() {
        let updates = Updates::default();
        assert!(!updates.prevents_exit(Some(0)));
        updates.inner.lock().unwrap().view.phase = Phase::Installing;
        assert!(updates.prevents_exit(None));
        assert!(updates.prevents_exit(Some(0)));
        assert!(!updates.prevents_exit(Some(tauri::RESTART_EXIT_CODE)));
        updates.inner.lock().unwrap().view.phase = Phase::InstallFailed;
        assert!(!updates.prevents_exit(Some(0)));
    }
}
