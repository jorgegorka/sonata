use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::webview::WebviewBuilder;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Url, Webview, WebviewUrl,
};

use crate::browser::{self, failed, Browser, PanelRect, MAIN_WINDOW};
use crate::commands::CommandError;
use crate::menu;

pub const WORLD: &str = "sonata-pick";
pub const HANDLER: &str = "sonataPick";
pub const PICK_EVENT: &str = "browser-pick";
pub const MODE_EVENT: &str = "browser-pick-mode";
pub const REFUSED_EVENT: &str = "browser-pick-refused";
pub const BOX_LABEL: &str = "pickbox";
pub const BOX_STATE_EVENT: &str = "pickbox-state";
const BOX_PAGE: &str = "pickbox.html";
const BOX_WIDTH: f64 = 360.0;
const BOX_HEIGHT: f64 = 184.0;
const BOX_GAP: f64 = 8.0;
const BOX_MARGIN: f64 = 8.0;
const SOURCE: &str = include_str!("browser_pick.js");

pub const STOP: &str = "globalThis.__sonataPick?.stop();0";

const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_COMMENT: usize = 4_000;
const MAX_URL: usize = 8_192;
const MAX_TITLE: usize = 300;
const MAX_SELECTOR: usize = 4_000;
const MAX_TEXT: usize = 1_000;
const MAX_HTML: usize = 3_000;
const MAX_TAG: usize = 64;
const MAX_RECT: f64 = 1_000_000.0;
const MAX_LOGGED_REASON: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickStrings {
    pub hint: String,
    pub placeholder: String,
    pub keys: String,
    pub send: String,
    pub sending: String,
    pub failed: String,
    pub no_target: String,
    pub too_long: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickedElement {
    pub url: String,
    pub title: String,
    pub selector: String,
    pub tag: String,
    pub text: String,
    pub html: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrowserPick {
    pub comment: String,
    pub element: PickedElement,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum PageMessage {
    Picked { element: PickedElement, rect: PageRect },
    Moved { rect: PageRect },
    Close {},
    Ended {},
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameOrigin {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BoxStatus {
    #[default]
    Idle,
    Sending,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickBoxState {
    pub target: Option<String>,
    pub strings: Option<PickStrings>,
    pub status: BoxStatus,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Closed,
    Kept,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl From<PanelRect> for PanelBounds {
    fn from(rect: PanelRect) -> Self {
        PanelBounds { x: rect.x, y: rect.y, width: rect.width.max(0.0), height: rect.height.max(0.0) }
    }
}

#[derive(Debug, Default)]
pub struct PickState {
    pub active: bool,
    pub target: Option<String>,
    pub strings: Option<PickStrings>,
    pub picked: Option<PickedElement>,
    pub rect: Option<PageRect>,
    pub panel: Option<PanelBounds>,
    pub status: BoxStatus,
    pub generation: u64,
    picks: u64,
    sending: u64,
}

impl PickState {
    pub fn pick(&mut self, element: PickedElement, rect: PageRect) -> bool {
        if !self.active {
            return false;
        }
        self.forget_failure();
        self.picked = Some(element);
        self.rect = Some(rect);
        self.picks += 1;
        true
    }

    pub fn moved(&mut self, rect: PageRect) -> bool {
        if self.picked.is_none() {
            return false;
        }
        self.rect = Some(rect);
        true
    }

    pub fn placed(&mut self, panel: PanelBounds) -> bool {
        self.panel = Some(panel);
        self.picked.is_some()
    }

    pub fn submit(&mut self, comment: &str) -> Result<BrowserPick, String> {
        let element = self.picked.clone().ok_or("no element is picked")?;
        if self.target.is_none() {
            return Err("there is no chat to send to".to_string());
        }
        if self.status == BoxStatus::Sending {
            return Err("a comment is being sent".to_string());
        }
        let comment = comment.trim();
        within("comment", comment, 1, MAX_COMMENT)?;
        self.status = BoxStatus::Sending;
        self.sending = self.picks;
        Ok(BrowserPick { comment: comment.to_string(), element })
    }

    pub fn result(&mut self, ok: bool) -> Option<Sent> {
        if self.status != BoxStatus::Sending {
            return None;
        }
        if !ok {
            self.status = BoxStatus::Failed;
            return Some(Sent::Failed);
        }
        self.status = BoxStatus::Idle;
        self.generation += 1;
        if self.picks == self.sending && self.close() {
            Some(Sent::Closed)
        } else {
            Some(Sent::Kept)
        }
    }

    pub fn drop_sending(&mut self) -> bool {
        let sending = self.status == BoxStatus::Sending;
        if sending {
            self.status = BoxStatus::Idle;
        }
        sending
    }

    pub fn close(&mut self) -> bool {
        self.rect = None;
        self.forget_failure();
        self.picked.take().is_some()
    }

    pub fn deactivate(&mut self) -> (bool, bool, bool) {
        let ended = (std::mem::replace(&mut self.active, false), self.close(), self.drop_sending());
        if ended.0 {
            self.generation += 1;
        }
        ended
    }

    fn forget_failure(&mut self) {
        if self.status == BoxStatus::Failed {
            self.status = BoxStatus::Idle;
        }
    }

    pub fn box_frame(&self) -> Option<((f64, f64), (f64, f64))> {
        self.picked.as_ref()?;
        let (panel, rect) = (self.panel?, self.rect?);
        let size = box_size(panel);
        Some((box_origin(panel, rect, size), size))
    }

    pub fn box_state(&self) -> PickBoxState {
        PickBoxState {
            target: self.target.clone(),
            strings: self.strings.clone(),
            status: self.status,
            generation: self.generation,
        }
    }

    pub fn menu_enabled(&self, has_page: bool) -> bool {
        self.active || (has_page && self.target.is_some())
    }
}

pub fn box_size(panel: PanelBounds) -> (f64, f64) {
    (BOX_WIDTH.min(panel.width - 2.0 * BOX_MARGIN).max(0.0), BOX_HEIGHT)
}

pub fn box_origin(panel: PanelBounds, rect: PageRect, (width, height): (f64, f64)) -> (f64, f64) {
    let (left, right) = (panel.x + BOX_MARGIN, panel.x + panel.width - BOX_MARGIN - width);
    let (top, bottom) = (panel.y + BOX_MARGIN, panel.y + panel.height - BOX_MARGIN - height);
    let below = panel.y + rect.y + rect.height + BOX_GAP;
    let above = panel.y + rect.y - BOX_GAP - height;
    let inside = |y: f64| top <= y && y <= bottom;
    let y = if inside(below) {
        below
    } else if inside(above) {
        above
    } else {
        below.min(bottom).max(top)
    };
    ((panel.x + rect.x).min(right).max(left), y)
}

pub fn parse_message(body: &str, origin: &FrameOrigin) -> Result<PageMessage, String> {
    if body.len() > MAX_BODY_BYTES {
        return Err(format!("message of {} bytes is over the limit", body.len()));
    }
    let message: PageMessage = serde_json::from_str(body).map_err(|e| format!("not a picker message: {e}"))?;
    match &message {
        PageMessage::Picked { element, rect } => {
            check_rect(rect)?;
            check_element(element, origin)?;
        }
        PageMessage::Moved { rect } => check_rect(rect)?,
        PageMessage::Close {} | PageMessage::Ended {} => {}
    }
    Ok(message)
}

fn check_rect(rect: &PageRect) -> Result<(), String> {
    let values = [rect.x, rect.y, rect.width, rect.height];
    if !values.iter().all(|v| v.is_finite() && v.abs() <= MAX_RECT) || rect.width < 0.0 || rect.height < 0.0 {
        return Err("rect is out of range".to_string());
    }
    Ok(())
}

fn check_element(element: &PickedElement, origin: &FrameOrigin) -> Result<(), String> {
    within("url", &element.url, 0, MAX_URL)?;
    within("title", &element.title, 0, MAX_TITLE)?;
    within("selector", &element.selector, 1, MAX_SELECTOR)?;
    within("text", &element.text, 0, MAX_TEXT)?;
    within("html", &element.html, 0, MAX_HTML)?;
    if !valid_tag(&element.tag) {
        return Err("tag is not an element name".to_string());
    }
    let url = Url::parse(&element.url).map_err(|e| format!("url does not parse: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("url is not a web page".to_string());
    }
    if !same_origin(&url, origin) {
        return Err("url is not from the frame that posted it".to_string());
    }
    Ok(())
}

fn within(field: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let n = value.chars().count();
    if n < min || n > max {
        return Err(format!("{field} has {n} characters, expected {min} to {max}"));
    }
    Ok(())
}

fn valid_tag(tag: &str) -> bool {
    let mut chars = tag.chars();
    matches!(chars.next(), Some('a'..='z'))
        && tag.len() <= MAX_TAG
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn same_origin(url: &Url, origin: &FrameOrigin) -> bool {
    let port = if origin.port == 0 {
        match origin.scheme.as_str() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        }
    } else {
        Some(origin.port)
    };
    url.scheme() == origin.scheme
        && url.host_str().is_some_and(|h| unbracketed(h).eq_ignore_ascii_case(unbracketed(&origin.host)))
        && port.is_some()
        && url.port_or_known_default() == port
}

fn unbracketed(host: &str) -> &str {
    host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host)
}

pub fn call(method: &str, arg: &impl Serialize) -> String {
    let json = serde_json::to_string(arg).expect("a picker argument serializes");
    format!("{SOURCE}\n;globalThis.__sonataPick.{method}({json});0")
}

fn for_log(reason: &str) -> String {
    let mut chars = reason.chars();
    let kept: String = chars.by_ref().take(MAX_LOGGED_REASON).collect();
    if chars.next().is_some() {
        kept + "…"
    } else {
        kept
    }
}

fn with_pick<T>(app: &AppHandle, f: impl FnOnce(&mut PickState) -> T) -> Option<T> {
    let browser = app.try_state::<Browser>()?;
    let mut pick = browser.pick.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    Some(f(&mut pick))
}

fn eval(panel: &Webview, script: String) -> tauri::Result<()> {
    let app = panel.app_handle().clone();
    panel.with_webview(move |pw| native::eval(app, pw, &script))
}

fn eval_in_panel(app: &AppHandle, script: String) {
    if let Some(panel) = browser::panel(app) {
        if let Err(e) = eval(&panel, script) {
            tracing::warn!(error = %e, "could not reach the picker in the browser panel");
        }
    }
}

fn focus_ui(app: &AppHandle) {
    if let Some(ui) = app.get_webview(MAIN_WINDOW) {
        let _ = ui.set_focus();
    }
}

fn focus_panel(app: &AppHandle) {
    if let Some(panel) = browser::panel(app) {
        if let Err(e) = panel.set_focus() {
            tracing::warn!(error = %e, "could not move typing to the browser panel");
        }
    }
}

fn on_main(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    let queue = move || {
        let on_it = app.clone();
        if let Err(e) = app.run_on_main_thread(move || f(&on_it)) {
            tracing::warn!(error = %e, "could not reach the pick comment box");
        }
    };
    // Always queue from outside the native callback to avoid reentrant GTK/AppKit updates.
    tauri::async_runtime::spawn(async move { queue() });
}

fn update_box(app: &AppHandle, focus: bool) {
    on_main(app, move |app| show_box(app, focus));
}

fn show_box(app: &AppHandle, focus: bool) {
    let frame = with_pick(app, |p| p.box_frame()).flatten();
    let pickbox = app.get_webview(BOX_LABEL);
    let Some(((x, y), (width, height))) = frame else {
        if let Some(pickbox) = pickbox {
            if let Err(e) = pickbox.hide() {
                tracing::warn!(error = %e, "could not hide the pick comment box");
            }
        }
        return;
    };
    let (position, size) = (LogicalPosition::new(x, y), LogicalSize::new(width, height));
    let shown = match pickbox {
        Some(pickbox) => {
            let bounds = Rect { position: Position::Logical(position), size: Size::Logical(size) };
            pickbox.set_bounds(bounds).and_then(|()| pickbox.show()).map(|()| pickbox)
        }
        None => make_box(app, position, size),
    };
    match shown {
        Ok(pickbox) if focus => {
            if let Err(e) = pickbox.set_focus() {
                tracing::warn!(error = %e, "could not move typing to the pick comment box");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "could not show the pick comment box"),
    }
}

fn make_box(app: &AppHandle, position: LogicalPosition<f64>, size: LogicalSize<f64>) -> tauri::Result<Webview> {
    let window = app.get_window(MAIN_WINDOW).ok_or(tauri::Error::WindowNotFound)?;
    let app_url = app.get_webview(MAIN_WINDOW).ok_or(tauri::Error::WebviewNotFound)?.url()?;
    let builder = WebviewBuilder::new(BOX_LABEL, WebviewUrl::App(BOX_PAGE.into()))
        .transparent(true)
        .focused(false)
        .on_navigation(move |url| {
            let own = is_box_page(url, &app_url);
            if !own {
                tracing::info!(url = %url, "the pick comment box refused a navigation");
            }
            own
        });
    window.add_child(builder, position, size)
}

pub fn is_box_page(url: &Url, app: &Url) -> bool {
    url.scheme() == app.scheme()
        && url.host_str() == app.host_str()
        && url.port_or_known_default() == app.port_or_known_default()
        && url.path() == format!("/{BOX_PAGE}")
}

fn send_box_state(app: &AppHandle) {
    on_main(app, |app| {
        if app.get_webview(BOX_LABEL).is_none() {
            return;
        }
        let Some(state) = with_pick(app, |p| p.box_state()) else { return };
        if let Err(e) = app.emit_to(BOX_LABEL, BOX_STATE_EVENT, state) {
            tracing::warn!(error = %e, "could not tell the pick comment box what it shows");
        }
    });
}

pub(crate) fn register(app: &AppHandle, panel: &Webview) {
    let app = app.clone();
    if let Err(e) = panel.with_webview(move |pw| native::register(app, pw)) {
        tracing::warn!(error = %e, "could not set up the picker in the browser panel");
    }
}

pub fn start(app: &AppHandle, strings: PickStrings, target: Option<String>) -> Result<(), CommandError> {
    let browser = app.state::<Browser>();
    let _lifecycle = browser.lifecycle.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    #[cfg(target_os = "macos")]
    if !objc2::available!(macos = 11.0) {
        return Err(failed("picking an element needs macOS 11 or later"));
    }
    let panel = browser::panel(app).ok_or_else(|| failed("the browser panel is not open"))?;
    if !browser.shown.load(Ordering::Relaxed) || !browser::has_page(app) {
        return Err(failed("the browser panel shows no page"));
    }
    let script = call("start", &json!({ "strings": strings, "target": target }));
    let was_on = with_pick(app, |p| {
        p.strings = Some(strings);
        p.target = target;
        let was_on = std::mem::replace(&mut p.active, true);
        if !was_on {
            p.drop_sending();
        }
        was_on
    })
    .unwrap_or(false);
    if let Err(e) = eval(&panel, script) {
        with_pick(app, |p| p.active = was_on);
        return Err(failed(e));
    }
    if !was_on {
        focus_panel(app);
    }
    set_active(app, true);
    send_box_state(app);
    Ok(())
}

pub fn stop(app: &AppHandle) -> Result<(), CommandError> {
    let browser = app.state::<Browser>();
    let _lifecycle = browser.lifecycle.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    off(app);
    Ok(())
}

pub(crate) fn off(app: &AppHandle) {
    if deactivate(app) {
        focus_ui(app);
        set_active(app, false);
    }
}

fn deactivate(app: &AppHandle) -> bool {
    let (was_on, had_box, was_sending) = with_pick(app, |p| p.deactivate()).unwrap_or((false, false, false));
    if was_on {
        eval_in_panel(app, STOP.to_string());
    }
    if had_box {
        update_box(app, false);
    }
    if was_on || was_sending {
        send_box_state(app);
    }
    was_on
}

pub fn set_target(app: &AppHandle, target: Option<String>) -> Result<(), CommandError> {
    with_pick(app, |p| p.target = target);
    send_box_state(app);
    refresh_menu(app);
    Ok(())
}

pub fn box_state(app: &AppHandle) -> PickBoxState {
    with_pick(app, |p| p.box_state()).unwrap_or_default()
}

pub fn submit(app: &AppHandle, comment: &str) -> Result<(), CommandError> {
    let pick = with_pick(app, |p| p.submit(comment)).ok_or_else(|| failed("the browser panel is gone"))?;
    let pick = pick.map_err(failed)?;
    send_box_state(app);
    if let Err(e) = app.emit_to(MAIN_WINDOW, PICK_EVENT, pick) {
        with_pick(app, |p| p.result(false));
        send_box_state(app);
        return Err(failed(e));
    }
    Ok(())
}

pub fn report(app: &AppHandle, ok: bool) -> Result<(), CommandError> {
    let Some(sent) = with_pick(app, |p| p.result(ok)).flatten() else {
        return Ok(());
    };
    if sent == Sent::Closed {
        update_box(app, false);
        eval_in_panel(app, call("sent", &()));
        focus_panel(app);
    }
    send_box_state(app);
    Ok(())
}

pub fn close_box(app: &AppHandle) -> Result<(), CommandError> {
    if with_pick(app, |p| p.close()).unwrap_or(false) {
        update_box(app, false);
        send_box_state(app);
        eval_in_panel(app, call("closed", &()));
        focus_panel(app);
    }
    Ok(())
}

pub(crate) fn panel_placed(app: &AppHandle, bounds: PanelBounds) {
    if with_pick(app, |p| p.placed(bounds)).unwrap_or(false) {
        update_box(app, false);
    }
}

pub(crate) fn receive(app: &AppHandle, body: &str, origin: &FrameOrigin) {
    match parse_message(body, origin) {
        Ok(PageMessage::Picked { element, rect }) => {
            if with_pick(app, |p| p.pick(element, rect)).unwrap_or(false) {
                send_box_state(app);
                update_box(app, true);
            } else {
                refuse(app, "a pick while picking is off");
            }
        }
        Ok(PageMessage::Moved { rect }) => {
            if with_pick(app, |p| p.moved(rect)).unwrap_or(false) {
                update_box(app, false);
            }
        }
        Ok(PageMessage::Close {}) => {
            if with_pick(app, |p| p.close()).unwrap_or(false) {
                update_box(app, false);
                send_box_state(app);
                eval_in_panel(app, call("closed", &()));
            }
        }
        Ok(PageMessage::Ended {}) => {
            if deactivate(app) {
                let for_focus = app.clone();
                tauri::async_runtime::spawn(async move { focus_ui(&for_focus) });
                set_active(app, false);
            }
        }
        Err(reason) => refuse(app, &reason),
    }
}

pub(crate) fn refuse(app: &AppHandle, reason: &str) {
    tracing::warn!(reason = ?for_log(reason), "the browser panel refused a message from the picker");
    if with_pick(app, |p| p.close()).unwrap_or(false) {
        update_box(app, false);
        send_box_state(app);
    }
    eval_in_panel(app, call("closed", &()));
    if let Err(e) = app.emit_to(MAIN_WINDOW, REFUSED_EVENT, ()) {
        tracing::warn!(error = %e, "could not tell the UI that the panel refused an element");
    }
}

pub(crate) fn page_loaded(app: &AppHandle, panel: &Webview, finished: bool) {
    let options = with_pick(app, |p| match (p.active, &p.strings) {
        (true, Some(strings)) => Some(json!({ "strings": strings, "target": p.target })),
        _ => None,
    });
    if let Some(options) = options.flatten() {
        if let Err(e) = eval(panel, call("start", &options)) {
            tracing::warn!(error = %e, "could not bring the picker back on the new page");
        }
    }
    if finished {
        refresh_menu(app);
    }
}

fn set_active(app: &AppHandle, active: bool) {
    with_pick(app, |p| p.active = active);
    if let Err(e) = app.emit_to(MAIN_WINDOW, MODE_EVENT, json!({ "active": active })) {
        tracing::warn!(error = %e, "could not tell the UI about pick mode");
    }
    refresh_menu(app);
}

pub(crate) fn refresh_menu(app: &AppHandle) {
    let has_page = browser::has_page(app);
    let Some((active, enabled)) = with_pick(app, |p| (p.active, p.menu_enabled(has_page))) else {
        return;
    };
    if let Err(e) = menu::set_pick(app, active, enabled) {
        tracing::warn!(error = %e, "could not update the pick item of the View menu");
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::panic::AssertUnwindSafe;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
    use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_foundation::{ns_string, NSError, NSObjectProtocol, NSString, NSUTF8StringEncoding};
    use objc2_web_kit::{WKContentWorld, WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKWebView};
    use tauri::webview::PlatformWebview;
    use tauri::AppHandle;

    use super::{for_log, receive, refuse, FrameOrigin, HANDLER, WORLD};

    struct PickIvars {
        app: AppHandle,
        _world: Retained<WKContentWorld>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "SonataPickHandler"]
        #[ivars = PickIvars]
        struct PickHandler;

        unsafe impl NSObjectProtocol for PickHandler {}

        unsafe impl WKScriptMessageHandler for PickHandler {
            #[unsafe(method(userContentController:didReceiveScriptMessage:))]
            fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
                let app = &self.ivars().app;
                match read(message) {
                    Ok((body, origin)) => receive(app, &body, &origin),
                    Err(reason) => refuse(app, reason),
                }
            }
        }
    );

    impl PickHandler {
        fn new(app: AppHandle, world: Retained<WKContentWorld>, mtm: MainThreadMarker) -> Retained<Self> {
            let this = mtm.alloc::<Self>().set_ivars(PickIvars { app, _world: world });
            unsafe { msg_send![super(this), init] }
        }
    }

    fn read(message: &WKScriptMessage) -> Result<(String, FrameOrigin), &'static str> {
        let (frame, world, body) = unsafe { (message.frameInfo(), message.world(), message.body()) };
        if !unsafe { frame.isMainFrame() } {
            return Err("a message from a subframe");
        }
        if unsafe { world.name() }.and_then(|name| text(&name)).as_deref() != Some(WORLD) {
            return Err("a message from another script world");
        }
        let body = body.downcast::<NSString>().map_err(|_| "a message that is not a string")?;
        let body = text(&body).ok_or("a message that is not valid text")?;
        let (scheme, host, port) = unsafe {
            let origin = frame.securityOrigin();
            (origin.protocol(), origin.host(), origin.port())
        };
        let origin = FrameOrigin {
            scheme: text(&scheme).ok_or("a frame origin that is not valid text")?,
            host: text(&host).ok_or("a frame origin that is not valid text")?,
            port: u16::try_from(port).map_err(|_| "a frame origin without a valid port")?,
        };
        Ok((body, origin))
    }

    fn text(string: &NSString) -> Option<String> {
        (string.length() == 0 || string.lengthOfBytesUsingEncoding(NSUTF8StringEncoding) > 0)
            .then(|| string.to_string())
    }

    pub fn register(app: AppHandle, pw: PlatformWebview) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        if !objc2::available!(macos = 11.0) {
            return;
        }
        let Some(controller) = (unsafe { pw.controller().cast::<WKUserContentController>().as_ref() }) else {
            return;
        };
        let world = unsafe { WKContentWorld::worldWithName(ns_string!(WORLD), mtm) };
        let handler = PickHandler::new(app, world.clone(), mtm);
        let added = objc2::exception::catch(AssertUnwindSafe(|| unsafe {
            controller.addScriptMessageHandler_contentWorld_name(
                ProtocolObject::from_ref(&*handler),
                &world,
                ns_string!(HANDLER),
            )
        }));
        if let Err(exception) = added {
            tracing::warn!(?exception, "could not register the picker's message handler");
        }
    }

    pub fn eval(_app: AppHandle, pw: PlatformWebview, script: &str) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        if !objc2::available!(macos = 11.0) {
            return;
        }
        let Some(webview) = (unsafe { pw.inner().cast::<WKWebView>().as_ref() }) else { return };
        let world = unsafe { WKContentWorld::worldWithName(ns_string!(WORLD), mtm) };
        let done: RcBlock<dyn Fn(*mut AnyObject, *mut NSError)> =
            RcBlock::new(|_: *mut AnyObject, error: *mut NSError| {
                if let Some(error) = unsafe { error.as_ref() } {
                    let reason = text(&error.localizedDescription()).unwrap_or_default();
                    tracing::warn!(error = ?for_log(&reason), "the picker script failed in the browser panel");
                }
            });
        unsafe {
            webview.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                &NSString::from_str(script),
                None,
                &world,
                Some(&*done),
            )
        };
    }
}

#[cfg(target_os = "linux")]
mod native {
    use super::{for_log, receive, refuse, FrameOrigin, HANDLER, MAX_BODY_BYTES, WORLD};
    use javascriptcore::ValueExt;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };
    use tauri::{webview::PlatformWebview, AppHandle, Manager, Url};
    use webkit2gtk::{gio, glib::object::ObjectExt, LoadEvent, UserContentManagerExt, WebViewExt};

    #[derive(Default)]
    pub struct Navigation(pub Arc<AtomicU64>);

    pub fn register(app: AppHandle, pw: PlatformWebview) {
        let view = pw.inner();
        let Some(manager) = view.user_content_manager() else { return };
        app.manage(Navigation::default());
        let epoch = app.state::<Navigation>().0.clone();
        epoch.fetch_add(1, Ordering::Relaxed);
        let loading_epoch = epoch.clone();
        view.connect_load_changed(move |_, event| {
            if event == LoadEvent::Started {
                loading_epoch.fetch_add(1, Ordering::Relaxed);
            }
        });
        let weak_view = view.downgrade();
        manager.connect_script_message_received(Some(HANDLER), move |_, result| {
            let Some(view) = weak_view.upgrade() else { return };
            let Some(value) = result.js_value().filter(|v| v.is_string()) else {
                refuse(&app, "a message that is not a string");
                return;
            };
            let body = value.to_str();
            if body.len() > MAX_BODY_BYTES + 1024 {
                refuse(&app, "a picker message that is too large");
                return;
            }
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Envelope {
                generation: u64,
                body: String,
            }
            let Ok(message) = serde_json::from_str::<Envelope>(&body) else {
                refuse(&app, "invalid picker transport");
                return;
            };
            if view.is_loading() || message.generation != epoch.load(Ordering::Relaxed) {
                return;
            }
            let Some(url) = view.uri().and_then(|uri| Url::parse(&uri).ok()) else { return };
            let origin = FrameOrigin {
                scheme: url.scheme().into(),
                host: url.host_str().unwrap_or_default().into(),
                port: url.port_or_known_default().unwrap_or(0),
            };
            receive(&app, &message.body, &origin);
        });
        if !manager.register_script_message_handler_in_world(HANDLER, WORLD) {
            tracing::warn!("could not register the Linux picker message handler");
        }
    }

    pub fn eval(app: AppHandle, pw: PlatformWebview, script: &str) {
        let view = pw.inner();
        if view.is_loading() {
            return;
        }
        let Some(navigation) = app.try_state::<Navigation>() else { return };
        let generation = navigation.0.load(Ordering::Relaxed);
        // Only this isolated main-frame world gets the bridge; page scripts and iframes do not.
        let script = format!("if (window === window.top) {{ globalThis.__sonataPost = body => window.webkit.messageHandlers.{HANDLER}.postMessage(JSON.stringify({{generation:{generation},body}}));\n{script}\n}}");
        // Keep compatibility with WebKitGTK 2.38 on the Linux baseline.
        #[allow(deprecated)]
        view.run_javascript_in_world(&script, WORLD, None::<&gio::Cancellable>, |result| {
            if let Err(error) = result {
                tracing::warn!(error = ?for_log(&error.to_string()), "the picker script failed in the browser panel");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn origin(scheme: &str, host: &str, port: u16) -> FrameOrigin {
        FrameOrigin { scheme: scheme.into(), host: host.into(), port }
    }

    fn element(url: &str) -> serde_json::Value {
        json!({"url": url, "title": "Orders", "selector": "main > table td:nth-of-type(2)", "tag": "td",
               "text": "42", "html": "<td>42</td>"})
    }

    fn rect_json() -> serde_json::Value {
        json!({"x": 10.0, "y": 20.0, "width": 100.0, "height": 30.0})
    }

    fn picked_with(el: serde_json::Value) -> String {
        json!({"type": "picked", "element": el, "rect": rect_json()}).to_string()
    }

    fn picked(url: &str) -> String {
        picked_with(element(url))
    }

    fn with(field: &str, value: String) -> String {
        let mut el = element("http://localhost:3000/orders");
        el[field] = json!(value);
        picked_with(el)
    }

    fn local() -> FrameOrigin {
        origin("http", "localhost", 3000)
    }

    fn ok(body: &str) -> bool {
        parse_message(body, &local()).is_ok()
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> PageRect {
        PageRect { x, y, width, height }
    }

    fn el() -> PickedElement {
        PickedElement {
            url: "http://localhost:3000/orders".into(),
            title: "Orders".into(),
            selector: "td".into(),
            tag: "td".into(),
            text: "42".into(),
            html: "<td>42</td>".into(),
        }
    }

    fn other() -> PickedElement {
        PickedElement { selector: "th".into(), tag: "th".into(), ..el() }
    }

    fn on(target: Option<&str>) -> PickState {
        PickState { active: true, target: target.map(String::from), ..Default::default() }
    }

    fn picking() -> PickState {
        let mut state = on(Some("To @feature"));
        assert!(state.pick(el(), rect(40.0, 50.0, 100.0, 30.0)));
        state
    }

    #[test]
    fn a_good_pick_parses() {
        let m = parse_message(&picked("http://localhost:3000/orders"), &local()).unwrap();
        let PageMessage::Picked { element, rect: r } = m else { panic!("not a pick") };
        assert_eq!(element.tag, "td");
        assert_eq!(element.url, "http://localhost:3000/orders");
        assert_eq!(r, rect(10.0, 20.0, 100.0, 30.0));
    }

    #[test]
    fn moved_close_and_ended_parse() {
        let moved = r#"{"type":"moved","rect":{"x":-5,"y":0.5,"width":0,"height":12}}"#;
        assert_eq!(parse_message(moved, &local()), Ok(PageMessage::Moved { rect: rect(-5.0, 0.5, 0.0, 12.0) }));
        assert_eq!(parse_message(r#"{"type":"close"}"#, &local()), Ok(PageMessage::Close {}));
        assert_eq!(parse_message(r#"{"type":"ended"}"#, &local()), Ok(PageMessage::Ended {}));
    }

    #[test]
    fn a_rect_is_finite_within_a_million_and_never_negative_in_size() {
        let moved = |r: serde_json::Value| ok(&json!({"type": "moved", "rect": r}).to_string());
        let r = |x: f64, y: f64, w: f64, h: f64| json!({"x": x, "y": y, "width": w, "height": h});
        assert!(moved(r(1e6, -1e6, 1e6, 0.0)));
        assert!(moved(r(-1e6, 1e6, 0.0, 1e6)));
        assert!(!moved(r(1_000_001.0, 0.0, 1.0, 1.0)));
        assert!(!moved(r(0.0, -1_000_001.0, 1.0, 1.0)));
        assert!(!moved(r(0.0, 0.0, 1_000_001.0, 1.0)));
        assert!(!moved(r(0.0, 0.0, 1.0, 1_000_001.0)));
        assert!(!moved(r(0.0, 0.0, -1.0, 1.0)));
        assert!(!moved(r(0.0, 0.0, 1.0, -0.5)));
        assert!(!ok(r#"{"type":"moved","rect":{"x":1e400,"y":0,"width":1,"height":1}}"#));
        assert!(!moved(json!({"x": 0, "y": 0, "width": 1})));
        assert!(!moved(json!({"x": 0, "y": 0, "width": 1, "height": 1, "z": 1})));
        assert!(!moved(json!({"x": "0", "y": 0, "width": 1, "height": 1})));
        assert!(!ok(r#"{"type":"moved"}"#));
        let mut pick: serde_json::Value = serde_json::from_str(&picked("http://localhost:3000/")).unwrap();
        pick["rect"]["width"] = json!(-1);
        assert!(!ok(&pick.to_string()));
        pick["rect"] = json!(null);
        assert!(!ok(&pick.to_string()));
    }

    #[test]
    fn each_limit_is_enforced() {
        let a = |n: usize| "a".repeat(n);
        let url = |n: usize| format!("http://localhost:3000/{}", "a".repeat(n - "http://localhost:3000/".len()));
        assert!(ok(&with("url", url(8_192))));
        assert!(!ok(&with("url", url(8_193))));

        assert!(ok(&with("title", a(300))));
        assert!(!ok(&with("title", a(301))));
        assert!(!ok(&with("selector", String::new())));
        assert!(ok(&with("selector", a(1))));
        assert!(ok(&with("selector", a(4_000))));
        assert!(!ok(&with("selector", a(4_001))));
        assert!(ok(&with("text", a(1_000))));
        assert!(!ok(&with("text", a(1_001))));
        assert!(ok(&with("html", a(3_000))));
        assert!(!ok(&with("html", a(3_001))));

        let over = format!(r#"{{"type":"ended"}}{}"#, " ".repeat(65_536));
        assert!(over.len() > 65_536);
        assert!(parse_message(&over, &local()).is_err());
        let exact = format!(r#"{{"type":"ended"}}{}"#, " ".repeat(65_536 - 16));
        assert_eq!(exact.len(), 65_536);
        assert!(parse_message(&exact, &local()).is_ok());
    }

    #[test]
    fn limits_count_characters_not_bytes() {
        assert!(ok(&with("title", "😀".repeat(300))));
        assert!(!ok(&with("title", "😀".repeat(301))));
    }

    #[test]
    fn unknown_fields_and_types_are_refused() {
        let mut el = element("http://localhost:3000/");
        el["extra"] = json!("x");
        assert!(!ok(&picked_with(el)));
        let mut top: serde_json::Value = serde_json::from_str(&picked("http://localhost:3000/")).unwrap();
        top["extra"] = json!(1);
        assert!(!ok(&top.to_string()));
        assert!(!ok(r#"{"type":"ended","extra":1}"#));
        assert!(!ok(r#"{"type":"close","extra":1}"#));
        assert!(!ok(r#"{"type":"moved","rect":{"x":0,"y":0,"width":1,"height":1},"extra":1}"#));
        assert!(!ok(r#"{"type":"other"}"#));
        let old = json!({"type": "pick", "comment": "why", "element": element("http://localhost:3000/")});
        assert!(!ok(&old.to_string()));
        let mut with_comment: serde_json::Value = serde_json::from_str(&picked("http://localhost:3000/")).unwrap();
        with_comment["comment"] = json!("why");
        assert!(!ok(&with_comment.to_string()));
        let mut el = element("http://localhost:3000/");
        el["title"] = json!(5);
        assert!(!ok(&picked_with(el)));
        assert!(!ok("not json"));
    }

    #[test]
    fn only_web_pages_can_send() {
        for url in ["about:blank", "file:///x", "javascript:x"] {
            assert!(parse_message(&picked(url), &origin("about", "", 0)).is_err(), "{url}");
            assert!(!ok(&picked(url)), "{url}");
        }
    }

    #[test]
    fn the_url_must_come_from_the_frame_that_posted() {
        let pick = |url: &str, o: &FrameOrigin| parse_message(&picked(url), o).is_ok();
        assert!(!pick("http://evil.test:3000/", &local()));
        assert!(!pick("https://localhost:3000/", &local()));
        assert!(!pick("http://localhost:3001/", &local()));
        assert!(pick("https://a.test/x", &origin("https", "a.test", 0)));
        assert!(pick("https://a.test:443/x", &origin("https", "a.test", 0)));
        assert!(!pick("https://a.test:444/x", &origin("https", "a.test", 0)));
        assert!(pick("http://localhost:3000/", &local()));
    }

    #[test]
    fn an_ipv6_origin_matches_with_or_without_brackets() {
        let pick = |url: &str, o: &FrameOrigin| parse_message(&picked(url), o).is_ok();
        assert!(pick("http://[::1]:3000/", &origin("http", "::1", 3000)));
        assert!(pick("http://[::1]:3000/", &origin("http", "[::1]", 3000)));
        assert!(!pick("http://[::1]:3000/", &origin("http", "::2", 3000)));
        assert!(!pick("http://[::1]:3000/", &origin("http", "[::1", 3000)));
    }

    #[test]
    fn a_bad_tag_is_refused() {
        for tag in ["Button", "1a", "a b", "", &"a".repeat(65)] {
            assert!(!ok(&with("tag", tag.to_string())), "{tag:?}");
        }
        for tag in ["my-widget", "h1", "element", &"a".repeat(64)] {
            assert!(ok(&with("tag", tag.to_string())), "{tag:?}");
        }
    }

    #[test]
    fn a_pick_is_kept_only_while_picking_is_on() {
        let mut off = PickState { target: Some("To @feature".into()), ..Default::default() };
        assert!(!off.pick(el(), rect(1.0, 2.0, 3.0, 4.0)));
        assert_eq!((off.picked, off.rect), (None, None));
        let mut state = on(None);
        assert!(state.pick(el(), rect(1.0, 2.0, 3.0, 4.0)));
        assert_eq!(state.picked.as_ref(), Some(&el()));
        assert!(state.pick(other(), rect(5.0, 6.0, 7.0, 8.0)));
        assert_eq!((state.picked, state.rect), (Some(other()), Some(rect(5.0, 6.0, 7.0, 8.0))));
    }

    #[test]
    fn a_move_is_kept_only_while_an_element_is_picked() {
        let mut state = on(Some("To @feature"));
        assert!(!state.moved(rect(1.0, 2.0, 3.0, 4.0)));
        assert_eq!(state.rect, None);
        let mut state = picking();
        assert!(state.moved(rect(1.0, 2.0, 3.0, 4.0)));
        assert_eq!(state.rect, Some(rect(1.0, 2.0, 3.0, 4.0)));
    }

    #[test]
    fn a_comment_goes_only_with_an_element_to_a_chat_and_one_at_a_time() {
        let mut nothing_picked = on(Some("To @feature"));
        assert!(nothing_picked.submit("why").is_err());
        let mut no_chat = on(None);
        no_chat.pick(el(), rect(1.0, 2.0, 3.0, 4.0));
        assert!(no_chat.submit("why").is_err());
        assert_eq!(no_chat.status, BoxStatus::Idle);

        let mut state = picking();
        assert_eq!(
            state.submit("  why is this 42?\n"),
            Ok(BrowserPick { comment: "why is this 42?".into(), element: el() })
        );
        assert_eq!(state.status, BoxStatus::Sending);
        assert!(state.submit("again").is_err(), "a second Enter while sending");
        assert_eq!(state.status, BoxStatus::Sending);
        assert_eq!(state.result(false), Some(Sent::Failed));
        assert_eq!(state.submit("again").map(|pick| pick.comment), Ok("again".to_string()));
    }

    #[test]
    fn a_comment_is_trimmed_and_holds_one_to_4000_characters() {
        let send = |comment: &str| picking().submit(comment).map(|pick| pick.comment.chars().count());
        assert!(send("   \n").is_err());
        assert!(send("").is_err());
        assert_eq!(send(&"a".repeat(4_000)), Ok(4_000));
        assert!(send(&"a".repeat(4_001)).is_err());
        assert_eq!(send(&format!("  {}\n", "a".repeat(4_000))), Ok(4_000));
        assert_eq!(send(&"😀".repeat(4_000)), Ok(4_000));
        assert!(send(&"😀".repeat(4_001)).is_err());
    }

    #[test]
    fn a_sent_comment_closes_the_box_and_clears_its_text() {
        let mut state = picking();
        state.submit("why").unwrap();
        assert_eq!(state.result(true), Some(Sent::Closed));
        assert_eq!((state.picked, state.rect, state.status, state.generation), (None, None, BoxStatus::Idle, 1));
        assert!(state.active, "pick mode stays on after a send");
    }

    #[test]
    fn a_comment_not_sent_keeps_the_box_and_says_so() {
        let mut state = picking();
        state.submit("why").unwrap();
        assert_eq!(state.result(false), Some(Sent::Failed));
        assert_eq!((state.picked, state.status, state.generation), (Some(el()), BoxStatus::Failed, 0));
    }

    #[test]
    fn a_comment_left_sending_is_dropped_so_the_next_one_can_go() {
        let mut state = picking();
        state.submit("why").unwrap();
        assert!(state.submit("again").is_err());
        assert!(state.drop_sending());
        assert_eq!(state.status, BoxStatus::Idle);
        assert!(!state.drop_sending(), "nothing to drop now");
        assert!(state.submit("again").is_ok());
        assert!(state.drop_sending());
        assert_eq!(state.result(true), None, "a late answer is ignored");
        assert_eq!((state.status, state.generation), (BoxStatus::Idle, 0));
        let mut failed = picking();
        failed.submit("why").unwrap();
        failed.result(false);
        assert!(!failed.drop_sending(), "a comment not sent keeps its text and its message");
        assert_eq!(failed.status, BoxStatus::Failed);
    }

    #[test]
    fn a_result_counts_only_while_a_comment_is_being_sent() {
        let mut state = picking();
        assert_eq!(state.result(true), None);
        assert_eq!(state.result(false), None);
        assert_eq!((state.picked.is_some(), state.status, state.generation), (true, BoxStatus::Idle, 0));
        state.submit("why").unwrap();
        state.result(true);
        assert_eq!(state.result(true), None, "a second answer");
        assert_eq!(state.generation, 1);
        let mut failed = picking();
        failed.submit("why").unwrap();
        failed.result(false);
        assert_eq!(failed.result(false), None);
        assert_eq!(failed.result(true), None);
        assert_eq!((failed.status, failed.generation), (BoxStatus::Failed, 0));
    }

    #[test]
    fn a_sent_comment_leaves_an_element_picked_since_in_its_box() {
        let mut state = picking();
        state.submit("why").unwrap();
        state.pick(other(), rect(5.0, 6.0, 7.0, 8.0));
        assert_eq!(state.result(true), Some(Sent::Kept));
        assert_eq!((state.picked, state.status, state.generation), (Some(other()), BoxStatus::Idle, 1));

        let mut state = picking();
        state.submit("why").unwrap();
        assert!(state.close());
        state.pick(el(), rect(40.0, 50.0, 100.0, 30.0));
        assert_eq!(state.result(true), Some(Sent::Kept));
        assert_eq!((state.picked.is_some(), state.generation), (true, 1));

        let mut state = picking();
        state.submit("why").unwrap();
        state.close();
        assert_eq!(state.result(true), Some(Sent::Kept));
        assert_eq!((state.picked, state.status, state.generation), (None, BoxStatus::Idle, 1));
    }

    #[test]
    fn closing_the_box_drops_the_element() {
        let mut state = on(Some("To @feature"));
        assert!(!state.close());
        let mut state = picking();
        assert!(state.close());
        assert!(state.picked.is_none() && state.rect.is_none());
        assert!(state.submit("why").is_err());
        assert!(!state.close());
    }

    fn not_sent() -> PickState {
        let mut state = picking();
        state.submit("why").unwrap();
        assert_eq!(state.result(false), Some(Sent::Failed));
        state
    }

    #[test]
    fn a_comment_not_sent_is_no_longer_said_to_be_once_the_box_closes_or_another_element_is_picked() {
        let mut state = not_sent();
        assert!(state.close());
        assert_eq!(state.status, BoxStatus::Idle);
        assert!(state.pick(el(), rect(40.0, 50.0, 100.0, 30.0)));
        assert_eq!(state.status, BoxStatus::Idle);

        let mut state = not_sent();
        assert!(state.pick(other(), rect(5.0, 6.0, 7.0, 8.0)));
        assert_eq!(state.status, BoxStatus::Idle);

        let mut state = picking();
        state.submit("why").unwrap();
        assert!(state.close());
        assert_eq!(state.status, BoxStatus::Sending);
        assert!(state.pick(el(), rect(40.0, 50.0, 100.0, 30.0)));
        assert_eq!(state.status, BoxStatus::Sending);
    }

    #[test]
    fn the_text_stays_across_esc_and_new_picks_and_goes_when_picking_ends() {
        let mut state = picking();
        assert!(state.close());
        assert!(state.pick(other(), rect(5.0, 6.0, 7.0, 8.0)));
        assert!(state.pick(el(), rect(40.0, 50.0, 100.0, 30.0)));
        assert_eq!(state.generation, 0, "Esc and new picks keep the text");

        assert_eq!(state.deactivate(), (true, true, false));
        assert_eq!((state.active, state.picked.is_none(), state.rect), (false, true, None));
        assert_eq!((state.status, state.generation), (BoxStatus::Idle, 1));
        assert_eq!(state.deactivate(), (false, false, false));
        assert_eq!(state.generation, 1);
        let mut state = picking();
        state.close();
        assert_eq!(state.deactivate(), (true, false, false));
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn ending_pick_mode_forgets_a_comment_sending_or_not_sent() {
        let mut sending = picking();
        sending.submit("why").unwrap();
        assert_eq!(sending.deactivate(), (true, true, true));
        assert_eq!((sending.status, sending.generation), (BoxStatus::Idle, 1));
        assert_eq!(sending.result(true), None, "a late answer is ignored");

        let mut failed = not_sent();
        assert_eq!(failed.deactivate(), (true, true, false));
        assert_eq!((failed.status, failed.generation), (BoxStatus::Idle, 1));
    }

    fn strings() -> PickStrings {
        PickStrings {
            hint: "Click to pick".into(),
            placeholder: "What about it?".into(),
            keys: "Enter to send".into(),
            send: "Send".into(),
            sending: "Sending…".into(),
            failed: "Not sent.".into(),
            no_target: "No chat to send to".into(),
            too_long: "Too long.".into(),
        }
    }

    #[test]
    fn the_box_state_names_the_chat_the_texts_the_status_and_the_generation() {
        let state = PickState {
            target: Some("To @feature · Login fix".into()),
            strings: Some(strings()),
            status: BoxStatus::Failed,
            generation: 3,
            ..picking()
        };
        let expected = json!({
            "target": "To @feature · Login fix",
            "strings": {"hint": "Click to pick", "placeholder": "What about it?", "keys": "Enter to send",
                        "send": "Send", "sending": "Sending…", "failed": "Not sent.", "noTarget": "No chat to send to",
                        "tooLong": "Too long."},
            "status": "failed",
            "generation": 3,
        });
        assert_eq!(serde_json::to_value(state.box_state()).unwrap(), expected);
        let idle = json!({"target": null, "strings": null, "status": "idle", "generation": 0});
        assert_eq!(serde_json::to_value(PickState::default().box_state()).unwrap(), idle);
        assert_eq!(serde_json::to_value(BoxStatus::Sending).unwrap(), json!("sending"));
    }

    fn panel() -> PanelBounds {
        PanelBounds { x: 600.0, y: 100.0, width: 500.0, height: 600.0 }
    }

    const SIZE: (f64, f64) = (360.0, 184.0);

    #[test]
    fn the_box_goes_below_the_element() {
        assert_eq!(box_origin(panel(), rect(40.0, 50.0, 100.0, 30.0), SIZE), (640.0, 188.0));
        assert_eq!(box_origin(panel(), rect(40.0, 300.0, 100.0, 100.0), SIZE), (640.0, 508.0));
    }

    #[test]
    fn the_box_goes_above_an_element_with_no_room_below() {
        assert_eq!(box_origin(panel(), rect(40.0, 450.0, 100.0, 50.0), SIZE), (640.0, 358.0));
        assert_eq!(box_origin(panel(), rect(40.0, 200.0, 100.0, 350.0), SIZE), (640.0, 108.0));
    }

    #[test]
    fn the_box_stays_inside_the_panel_with_a_margin() {
        assert_eq!(box_origin(panel(), rect(400.0, 50.0, 50.0, 30.0), SIZE), (732.0, 188.0));
        assert_eq!(box_origin(panel(), rect(-30.0, 50.0, 50.0, 30.0), SIZE), (608.0, 188.0));
        assert_eq!(box_origin(panel(), rect(40.0, -100.0, 100.0, 95.0), SIZE), (640.0, 108.0));
        assert_eq!(box_origin(panel(), rect(40.0, 100.0, 100.0, 550.0), SIZE), (640.0, 508.0));
        assert_eq!(box_origin(panel(), rect(0.0, 0.0, 500.0, 600.0), SIZE), (608.0, 508.0));
    }

    #[test]
    fn an_element_out_of_view_pins_the_box_to_the_nearest_edge() {
        assert_eq!(box_origin(panel(), rect(40.0, -500.0, 100.0, 30.0), SIZE), (640.0, 108.0), "above");
        assert_eq!(box_origin(panel(), rect(40.0, 900.0, 100.0, 30.0), SIZE), (640.0, 508.0), "below");
        assert_eq!(box_origin(panel(), rect(800.0, 50.0, 100.0, 30.0), SIZE), (732.0, 188.0), "right");
        assert_eq!(box_origin(panel(), rect(-400.0, 50.0, 100.0, 30.0), SIZE), (608.0, 188.0), "left");
        assert_eq!(box_origin(panel(), rect(-900.0, -900.0, 10.0, 10.0), SIZE), (608.0, 108.0), "above left");
    }

    #[test]
    fn the_box_is_never_wider_than_the_panel() {
        assert_eq!(box_size(panel()), SIZE);
        let narrow = PanelBounds { width: 320.0, ..panel() };
        assert_eq!(box_size(narrow), (304.0, 184.0));
        assert_eq!(box_origin(narrow, rect(200.0, 50.0, 100.0, 30.0), box_size(narrow)), (608.0, 188.0));
        assert_eq!(box_size(PanelBounds { width: 10.0, ..panel() }), (0.0, 184.0));
    }

    #[test]
    fn the_box_frame_follows_the_picked_element_and_the_panel() {
        let mut state = on(Some("To @feature"));
        assert!(!state.placed(panel()), "nothing picked: the box has nothing to follow");
        assert_eq!(state.box_frame(), None);
        state.pick(el(), rect(40.0, 50.0, 100.0, 30.0));
        assert_eq!(state.box_frame(), Some(((640.0, 188.0), SIZE)));
        state.moved(rect(40.0, 60.0, 100.0, 30.0));
        assert_eq!(state.box_frame(), Some(((640.0, 198.0), SIZE)));
        assert!(state.placed(PanelBounds { x: 500.0, ..panel() }));
        assert_eq!(state.box_frame(), Some(((540.0, 198.0), SIZE)));
        state.close();
        assert_eq!(state.box_frame(), None);
        assert_eq!(picking().box_frame(), None);
    }

    #[test]
    fn the_panel_bounds_are_the_rect_in_the_content_with_sizes_never_below_zero() {
        let rect =
            PanelRect { x: 580.0, y: 57.0, width: -3.0, height: 663.0, viewport_width: 1100.0, viewport_height: 688.0 };
        assert_eq!(PanelBounds::from(rect), PanelBounds { x: 580.0, y: 57.0, width: 0.0, height: 663.0 });
    }

    #[test]
    fn the_pickbox_capability_lets_the_box_hear_events_and_covers_nothing_else() {
        let capability: serde_json::Value = serde_json::from_str(include_str!("../capabilities/pickbox.json")).unwrap();
        assert_eq!(BOX_LABEL, "pickbox");
        assert_eq!(capability["webviews"], json!([BOX_LABEL]), "{capability}");
        assert!(capability.get("windows").is_none(), "{capability}");
        assert!(capability.get("remote").is_none(), "{capability}");
        assert_eq!(capability["permissions"], json!(["core:event:allow-listen", "core:event:allow-unlisten"]));
    }

    #[test]
    fn the_box_loads_only_its_own_page_of_the_app() {
        let app = |s: &str| Url::parse(s).unwrap();
        for own in [app("tauri://localhost/"), app("http://localhost:1420/index.html")] {
            let page = own.join("/pickbox.html").unwrap();
            assert!(is_box_page(&page, &own), "{page}");
            assert!(is_box_page(&own.join("/pickbox.html?x=1#top").unwrap(), &own));
            for other in ["/index.html", "/", "/pickbox.html/x", "/assets/pickbox.html", "/pickbox.htm"] {
                assert!(!is_box_page(&own.join(other).unwrap(), &own), "{other}");
            }
        }
        let own = app("tauri://localhost/");
        for url in [
            "https://example.com/pickbox.html",
            "http://localhost:1420/pickbox.html",
            "tauri://example.com/pickbox.html",
            "file:///pickbox.html",
            "about:blank",
            "data:text/html,hi",
        ] {
            assert!(!is_box_page(&app(url), &own), "{url}");
        }
        let dev = app("http://localhost:1420/");
        assert!(!is_box_page(&app("http://localhost:1421/pickbox.html"), &dev));
        assert!(!is_box_page(&app("https://localhost:1420/pickbox.html"), &dev));
    }

    #[test]
    fn the_menu_item_can_be_chosen_with_a_page_and_a_chat_to_send_to() {
        let to = |target: Option<&str>| PickState { target: target.map(String::from), ..Default::default() };
        assert!(to(Some("To @feature")).menu_enabled(true));
        assert!(!to(Some("To @feature")).menu_enabled(false));
        assert!(!to(None).menu_enabled(true));
        let on = PickState { active: true, ..to(Some("To @feature")) };
        assert!(on.menu_enabled(true));
    }

    #[test]
    fn stop_picking_can_always_be_chosen() {
        for has_page in [true, false] {
            assert!(on(None).menu_enabled(has_page), "no chat, has_page {has_page}");
            assert!(on(Some("To @feature")).menu_enabled(has_page), "a chat, has_page {has_page}");
        }
    }

    #[test]
    fn a_logged_reason_is_capped_and_its_newlines_escaped() {
        assert_eq!(for_log("url does not parse"), "url does not parse");
        let long = for_log(&"é".repeat(500));
        assert_eq!((long.chars().count(), long.ends_with('…')), (201, true));
        assert_eq!(for_log(&"a".repeat(200)), "a".repeat(200));
        let logged = format!("{:?}", for_log("unknown variant `x\ninjected line`"));
        assert!(!logged.contains('\n') && logged.contains("\\n"), "{logged}");
    }

    #[test]
    fn the_call_embeds_its_argument_as_json() {
        let hint = "To \"@a\" </script> \u{2028}";
        let script = call("start", &json!({ "strings": { "hint": hint } }));
        assert!(script.starts_with(SOURCE));
        let arg = serde_json::to_string(&json!({ "strings": { "hint": hint } })).unwrap();
        assert!(script.contains(&format!("__sonataPick.start({arg})")));
        assert!(script.ends_with(";0"));
    }
}
