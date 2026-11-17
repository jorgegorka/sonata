use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Url, Webview, WebviewUrl, Wry,
};
use tauri_plugin_opener::OpenerExt;

use crate::browser_pick::{self, PickState};
use crate::commands::CommandError;

pub const MAIN_WINDOW: &str = "main";
pub const LABEL: &str = "browser";
pub const STATE_EVENT: &str = "browser-state";
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) \
                              Version/17.6 Safari/605.1.15";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct PanelRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
}

impl PanelRect {
    pub fn in_content(self, content: LogicalSize<f64>) -> PanelRect {
        let dx = (content.width - self.viewport_width).max(0.0);
        let dy = (content.height - self.viewport_height).max(0.0);
        PanelRect { x: self.x + dx, y: self.y + dy, ..self }
    }

    pub fn bounds(self) -> Rect {
        Rect {
            position: Position::Logical(LogicalPosition::new(self.x, self.y)),
            size: Size::Logical(LogicalSize::new(self.width.max(0.0), self.height.max(0.0))),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PageState {
    pub url: String,
    pub title: String,
    pub loading: bool,
    #[serde(skip)]
    pub committed: String,
}

pub enum PageEvent {
    Requested(Url),
    Started(Url),
    Finished(Url),
    Title(String),
    Moved(Url),
    Download,
}

impl PageState {
    pub fn apply(&mut self, event: PageEvent) {
        match event {
            PageEvent::Requested(url) => {
                self.url = url.into();
                self.loading = true;
            }
            PageEvent::Started(url) => {
                self.url = url.into();
                self.committed.clone_from(&self.url);
                self.title.clear();
                self.loading = true;
            }
            PageEvent::Finished(url) => {
                self.url = url.into();
                self.committed.clone_from(&self.url);
                self.loading = false;
            }
            PageEvent::Title(title) => self.title = title,
            PageEvent::Moved(url) => {
                self.url = url.into();
                self.committed.clone_from(&self.url);
            }
            PageEvent::Download => {
                self.url.clone_from(&self.committed);
                self.loading = false;
            }
        }
    }
}

pub enum NavigationAction {
    Follow,
    OpenOutside,
    Block,
}

pub fn navigation_action(url: &Url) -> NavigationAction {
    match url.scheme() {
        "http" | "https" | "about" => NavigationAction::Follow,
        "mailto" | "tel" => NavigationAction::OpenOutside,
        _ => NavigationAction::Block,
    }
}

fn is_web(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host().is_some()
}

pub fn web_url(input: &str) -> Result<Url, CommandError> {
    Url::parse(input).ok().filter(is_web).ok_or_else(|| bad_url(input))
}

pub fn address_to_url(input: &str) -> Result<Url, CommandError> {
    let address = input.trim();
    if let Ok(url) = Url::parse(address) {
        if is_web(&url) {
            return Ok(url);
        }
        if address.contains("://") || !looks_like_host_and_port(address) {
            return Err(bad_url(address));
        }
    }
    let host = address.split([':', '/', '?', '#']).next().unwrap_or("");
    if !looks_like_host(host) {
        return Err(bad_url(address));
    }
    let scheme = if is_local_machine(host) { "http" } else { "https" };
    Url::parse(&format!("{scheme}://{address}")).ok().filter(is_web).ok_or_else(|| bad_url(address))
}

fn looks_like_host(host: &str) -> bool {
    is_local_machine(host) || host.contains('.') || host.starts_with('[')
}

fn is_local_machine(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

fn looks_like_host_and_port(address: &str) -> bool {
    let Some((_, rest)) = address.split_once(':') else { return false };
    let port = rest.split(['/', '?', '#']).next().unwrap_or("");
    !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())
}

#[derive(Debug, PartialEq)]
pub enum DownloadAction {
    OpenOutside(Url),
    Drop,
}

pub fn download_action(url: &Url) -> DownloadAction {
    if is_web(url) {
        DownloadAction::OpenOutside(url.clone())
    } else {
        DownloadAction::Drop
    }
}

#[derive(Debug, PartialEq)]
pub enum NewWindowAction {
    LoadHere(Url),
    Drop,
}

pub fn new_window_action(url: &Url) -> NewWindowAction {
    if is_web(url) {
        NewWindowAction::LoadHere(url.clone())
    } else {
        NewWindowAction::Drop
    }
}

fn bad_url(address: &str) -> CommandError {
    CommandError::new("browserBadUrl", json!({ "address": address }))
}

pub(crate) fn failed(error: impl std::fmt::Display) -> CommandError {
    CommandError::new("browserFailed", json!({ "error": error.to_string() }))
}

#[derive(Default)]
pub struct Browser {
    page: Mutex<PageState>,
    pub(crate) lifecycle: Mutex<()>,
    pub(crate) pick: Mutex<PickState>,
    pub(crate) shown: AtomicBool,
}

pub(crate) fn panel(app: &AppHandle) -> Option<Webview> {
    app.get_webview(LABEL)
}

pub(crate) fn has_page(app: &AppHandle) -> bool {
    app.try_state::<Browser>().is_some_and(|browser| {
        !browser.page.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).committed.is_empty()
    })
}

fn content_size(app: &AppHandle) -> Result<LogicalSize<f64>, CommandError> {
    let window = app.get_window(MAIN_WINDOW).ok_or_else(|| failed("the main window is gone"))?;
    let scale = window.scale_factor().map_err(failed)?;
    Ok(window.inner_size().map_err(failed)?.to_logical(scale))
}

fn publish(app: &AppHandle, events: impl IntoIterator<Item = PageEvent>) -> PageState {
    let state = {
        let browser = app.state::<Browser>();
        let mut page = browser.page.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for event in events {
            page.apply(event);
        }
        page.clone()
    };
    if let Err(e) = app.emit_to(MAIN_WINDOW, STATE_EVENT, state.clone()) {
        tracing::warn!(error = %e, "could not tell the UI about the browser page");
    }
    state
}

fn open_outside(app: &AppHandle, url: Url) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
            tracing::warn!(error = %e, url = %url, "could not open the link outside the app");
        }
    });
}

fn builder(app: &AppHandle, url: Url) -> WebviewBuilder<Wry> {
    let for_navigation = app.clone();
    let for_load = app.clone();
    let for_title = app.clone();
    let for_download = app.clone();
    let for_new_window = app.clone();
    WebviewBuilder::new(LABEL, WebviewUrl::External(url))
        .user_agent(USER_AGENT)
        .focused(false)
        .on_navigation(move |url| match navigation_action(url) {
            NavigationAction::Follow => true,
            NavigationAction::OpenOutside => {
                open_outside(&for_navigation, url.clone());
                false
            }
            NavigationAction::Block => {
                tracing::info!(url = %url, "the browser panel refused a navigation");
                false
            }
        })
        .on_page_load(move |webview, payload| {
            let url = payload.url().clone();
            let (event, finished) = match payload.event() {
                PageLoadEvent::Started => (PageEvent::Started(url), false),
                PageLoadEvent::Finished => (PageEvent::Finished(url), true),
            };
            publish(&for_load, [event]);
            browser_pick::page_loaded(&for_load, &webview, finished);
        })
        .on_document_title_changed(move |webview, title| {
            let moved = webview.url().ok().map(PageEvent::Moved);
            publish(&for_title, [PageEvent::Title(title)].into_iter().chain(moved));
        })
        .on_download(move |_, event| {
            if let DownloadEvent::Requested { url, .. } = event {
                match download_action(&url) {
                    DownloadAction::OpenOutside(url) => open_outside(&for_download, url),
                    DownloadAction::Drop => tracing::info!(url = %url, "the browser panel dropped a download"),
                }
                publish(&for_download, [PageEvent::Download]);
            }
            false
        })
        .on_new_window(move |url, _features| {
            if let NewWindowAction::LoadHere(url) = new_window_action(&url) {
                let app = for_new_window.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(panel) = panel(&app) {
                        if let Err(e) = panel.navigate(url) {
                            tracing::warn!(error = %e, "the browser panel could not load a new-window link");
                        }
                    }
                });
            }
            NewWindowResponse::Deny
        })
}

fn open_panel(app: &AppHandle) -> Result<Webview, CommandError> {
    panel(app).ok_or_else(|| CommandError::internal("the browser panel is not open"))
}

pub fn navigate(app: &AppHandle, address: &str, rect: PanelRect) -> Result<(), CommandError> {
    let url = address_to_url(address)?;
    match panel(app) {
        Some(panel) => {
            publish(app, [PageEvent::Requested(url.clone())]);
            panel.navigate(url).map_err(failed)
        }
        None => show_url(app, url, rect).map(|_| ()),
    }
}

pub fn back(app: &AppHandle) -> Result<(), CommandError> {
    open_panel(app)?.eval("history.back()").map_err(failed)
}

pub fn forward(app: &AppHandle) -> Result<(), CommandError> {
    open_panel(app)?.eval("history.forward()").map_err(failed)
}

pub fn reload(app: &AppHandle) -> Result<(), CommandError> {
    open_panel(app)?.reload().map_err(failed)
}

fn show_at(panel: &Webview, url: Url, bounds: Rect) -> Result<(), CommandError> {
    panel.set_bounds(bounds).map_err(failed)?;
    panel.navigate(url).map_err(failed)?;
    panel.show().map_err(failed)
}

pub fn open(app: &AppHandle, url: &str, rect: PanelRect) -> Result<PageState, CommandError> {
    show_url(app, web_url(url)?, rect)
}

fn show_url(app: &AppHandle, url: Url, rect: PanelRect) -> Result<PageState, CommandError> {
    let browser = app.state::<Browser>();
    let _lifecycle = browser.lifecycle.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = publish(app, [PageEvent::Requested(url.clone())]);
    let rect = rect.in_content(content_size(app)?);
    match panel(app) {
        Some(panel) => show_at(&panel, url, rect.bounds())?,
        None => {
            let window = app.get_window(MAIN_WINDOW).ok_or_else(|| failed("the main window is gone"))?;
            let size = LogicalSize::new(rect.width.max(0.0), rect.height.max(0.0));
            match window.add_child(builder(app, url.clone()), LogicalPosition::new(rect.x, rect.y), size) {
                Ok(webview) => browser_pick::register(app, &webview),
                Err(tauri::Error::WebviewLabelAlreadyExists(_)) => {
                    let panel = panel(app).ok_or_else(|| failed("the browser panel is gone"))?;
                    show_at(&panel, url, rect.bounds())?;
                }
                Err(e) => return Err(failed(e)),
            }
        }
    }
    browser.shown.store(true, Ordering::Relaxed);
    browser_pick::panel_placed(app, rect.into());
    Ok(state)
}

pub fn reveal(app: &AppHandle, rect: PanelRect) -> Result<Option<PageState>, CommandError> {
    let browser = app.state::<Browser>();
    let _lifecycle = browser.lifecycle.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(panel) = panel(app) else {
        return Ok(None);
    };
    let rect = rect.in_content(content_size(app)?);
    panel.set_bounds(rect.bounds()).map_err(failed)?;
    panel.show().map_err(failed)?;
    browser.shown.store(true, Ordering::Relaxed);
    browser_pick::panel_placed(app, rect.into());
    let page = browser.page.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    Ok(Some(page))
}

pub fn place(app: &AppHandle, rect: PanelRect) -> Result<(), CommandError> {
    let Some(panel) = panel(app) else {
        return Ok(());
    };
    let rect = rect.in_content(content_size(app)?);
    panel.set_bounds(rect.bounds()).map_err(failed)?;
    browser_pick::panel_placed(app, rect.into());
    Ok(())
}

pub fn close(app: &AppHandle) -> Result<(), CommandError> {
    let browser = app.state::<Browser>();
    let _lifecycle = browser.lifecycle.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    browser_pick::off(app);
    browser.shown.store(false, Ordering::Relaxed);
    if let Some(panel) = panel(app) {
        panel.hide().map_err(failed)?;
    }
    if let Some(ui) = app.get_webview(MAIN_WINDOW) {
        let _ = ui.set_focus();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_capability_covers_the_app_webview_only() {
        let capability: serde_json::Value = serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(capability["webviews"], serde_json::json!(["main"]), "{capability}");
        assert!(capability.get("windows").is_none(), "{capability}");
        assert!(capability.get("remote").is_none(), "{capability}");
    }

    #[test]
    fn only_web_addresses_open_in_the_panel() {
        let mr = "https://gitlab.com/sonata/sonata/-/merge_requests/1";
        assert_eq!(web_url(mr).unwrap().as_str(), mr);
        assert_eq!(web_url("http://localhost:3000/").unwrap().as_str(), "http://localhost:3000/");
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "tauri://localhost", "mailto:a@b.c", "not a url", ""] {
            let e = web_url(bad).unwrap_err();
            assert_eq!((e.code.as_str(), e.params["address"].as_str()), ("browserBadUrl", Some(bad)), "{bad}");
        }
    }

    #[test]
    fn the_panel_follows_web_pages_and_webkits_empty_frames_and_nothing_else() {
        let action = |u: &str| navigation_action(&Url::parse(u).unwrap());
        for followed in ["https://example.com/", "http://localhost:1420/", "about:blank", "about:srcdoc"] {
            assert!(matches!(action(followed), NavigationAction::Follow), "{followed}");
        }
        assert!(matches!(action("mailto:a@b.c"), NavigationAction::OpenOutside));
        assert!(matches!(action("tel:+4912345"), NavigationAction::OpenOutside));
        let blocked = [
            "file:///etc/passwd",
            "tauri://localhost/",
            "ipc://localhost/",
            "asset://localhost/x",
            "javascript:alert(1)",
            "data:text/html,hi",
            "blob:https://example.com/1",
        ];
        for url in blocked {
            assert!(matches!(action(url), NavigationAction::Block), "{url}");
        }
    }

    #[test]
    fn page_events_fold_into_the_state_the_toolbar_shows() {
        let url = |u: &str| Url::parse(u).unwrap();
        let mut page = PageState::default();
        let state = |url: &str, title: &str, loading: bool| PageState {
            url: url.into(),
            title: title.into(),
            loading,
            committed: url.into(),
        };
        page.apply(PageEvent::Started(url("https://a.test/")));
        assert_eq!(page, state("https://a.test/", "", true));
        page.apply(PageEvent::Title("A".into()));
        assert_eq!((page.title.as_str(), page.loading), ("A", true));
        page.apply(PageEvent::Finished(url("https://a.test/landed")));
        assert_eq!(page, state("https://a.test/landed", "A", false));
        page.apply(PageEvent::Moved(url("https://a.test/landed#tab")));
        assert_eq!(page, state("https://a.test/landed#tab", "A", false));
        page.apply(PageEvent::Started(url("https://b.test/")));
        assert_eq!(page, state("https://b.test/", "", true));
    }

    #[test]
    fn a_request_shows_the_new_address_but_commits_nothing() {
        let url = |u: &str| Url::parse(u).unwrap();
        let mut page = PageState::default();
        page.apply(PageEvent::Finished(url("https://a.test/")));
        page.apply(PageEvent::Requested(url("https://b.test/")));
        assert_eq!(
            (page.url.as_str(), page.loading, page.committed.as_str()),
            ("https://b.test/", true, "https://a.test/")
        );
        page.apply(PageEvent::Started(url("https://b.test/")));
        assert_eq!(page.committed, "https://b.test/");
    }

    #[test]
    fn a_download_puts_the_address_back_on_the_page_it_came_from() {
        let url = |u: &str| Url::parse(u).unwrap();
        let mut page = PageState::default();
        page.apply(PageEvent::Finished(url("https://a.test/")));
        page.apply(PageEvent::Title("A".into()));
        page.apply(PageEvent::Requested(url("https://a.test/report.zip")));
        page.apply(PageEvent::Download);
        assert_eq!((page.url.as_str(), page.title.as_str(), page.loading), ("https://a.test/", "A", false));
    }

    #[test]
    fn the_panel_rect_becomes_logical_bounds_with_sizes_never_below_zero() {
        let rect =
            PanelRect { x: 580.0, y: 57.0, width: -3.0, height: 663.0, viewport_width: 1100.0, viewport_height: 688.0 }
                .bounds();
        match (rect.position, rect.size) {
            (Position::Logical(p), Size::Logical(s)) => {
                assert_eq!((p.x, p.y, s.width, s.height), (580.0, 57.0, 0.0, 663.0))
            }
            other => panic!("physical bounds: {other:?}"),
        }
    }

    #[test]
    fn a_rect_measured_in_the_ui_viewport_is_moved_below_the_title_bar() {
        let rect = PanelRect {
            x: 647.0,
            y: 75.0,
            width: 473.0,
            height: 613.0,
            viewport_width: 1120.0,
            viewport_height: 688.0,
        };
        let under_title_bar = rect.in_content(LogicalSize::new(1120.0, 720.0));
        assert_eq!((under_title_bar.x, under_title_bar.y), (647.0, 107.0));
        let full_screen = rect.in_content(LogicalSize::new(1120.0, 688.0));
        assert_eq!((full_screen.x, full_screen.y), (647.0, 75.0));
        let smaller = rect.in_content(LogicalSize::new(1000.0, 600.0));
        assert_eq!((smaller.x, smaller.y), (647.0, 75.0));
    }

    #[test]
    fn the_state_event_carries_url_title_and_loading() {
        let page =
            PageState { url: "https://a.test/".into(), title: "A".into(), loading: false, committed: "x".into() };
        let expected = serde_json::json!({ "url": "https://a.test/", "title": "A", "loading": false });
        assert_eq!(serde_json::to_value(&page).unwrap(), expected);
    }

    #[test]
    fn an_address_without_a_scheme_gets_https() {
        assert_eq!(address_to_url("example.com").unwrap().as_str(), "https://example.com/");
        assert_eq!(
            address_to_url("  gitlab.com/sonata/sonata  ").unwrap().as_str(),
            "https://gitlab.com/sonata/sonata"
        );
        assert_eq!(address_to_url("http://a.test/x?y=1").unwrap().as_str(), "http://a.test/x?y=1");
    }

    #[test]
    fn a_local_address_without_a_scheme_gets_http_and_a_port_is_not_a_scheme() {
        assert_eq!(address_to_url("localhost:1420/x").unwrap().as_str(), "http://localhost:1420/x");
        assert_eq!(address_to_url("127.0.0.1:3000").unwrap().as_str(), "http://127.0.0.1:3000/");
        assert_eq!(address_to_url("example.com:8443/x").unwrap().as_str(), "https://example.com:8443/x");
    }

    #[test]
    fn an_address_that_is_not_a_web_page_is_refused() {
        for bad in ["mailto:a@b.c", "javascript:alert(1)", "file:///tmp", "a b", "", "foo", "foo/bar"] {
            let e = address_to_url(bad).unwrap_err();
            assert_eq!((e.code.as_str(), e.params["address"].as_str()), ("browserBadUrl", Some(bad)), "{bad}");
        }
    }

    #[test]
    fn a_download_of_a_web_file_opens_outside_and_a_page_made_file_is_dropped() {
        let url = |u: &str| Url::parse(u).unwrap();
        let pdf = url("https://a.test/report.pdf");
        assert_eq!(download_action(&pdf), DownloadAction::OpenOutside(pdf.clone()));
        assert_eq!(download_action(&url("blob:https://a.test/1")), DownloadAction::Drop);
        assert_eq!(download_action(&url("data:text/plain,hi")), DownloadAction::Drop);
    }

    #[test]
    fn a_new_window_for_a_web_page_loads_in_the_panel_and_anything_else_is_dropped() {
        let url = |u: &str| Url::parse(u).unwrap();
        let doc = url("https://a.test/doc");
        assert_eq!(new_window_action(&doc), NewWindowAction::LoadHere(doc.clone()));
        assert_eq!(new_window_action(&url("javascript:void(0)")), NewWindowAction::Drop);
        assert_eq!(new_window_action(&url("about:blank")), NewWindowAction::Drop);
    }

    #[test]
    fn the_user_agent_names_safari() {
        assert!(USER_AGENT.contains("Version/") && USER_AGENT.contains("Safari/"), "{USER_AGENT}");
    }
}
