use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID};
use tauri::{AppHandle, Manager, Runtime};

pub const SETTINGS_ID: &str = "settings";
pub const ABOUT_ID: &str = "about";
pub const BROWSER_ID: &str = "browser";
pub const PICK_ID: &str = "pick";
const VIEW_ID: &str = "view";

pub fn ui_event_for(id: &str) -> Option<&'static str> {
    match id {
        SETTINGS_ID => Some("open-settings"),
        ABOUT_ID => Some("open-about"),
        BROWSER_ID => Some("toggle-browser"),
        PICK_ID => Some("toggle-pick"),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct ViewState {
    browser_shown: AtomicBool,
    pick_active: AtomicBool,
    pick_enabled: AtomicBool,
}

pub fn browser_label(l: &Labels, shown: bool) -> &str {
    if shown {
        &l.hide_browser
    } else {
        &l.show_browser
    }
}

pub fn pick_label(l: &Labels, active: bool) -> &str {
    if active {
        &l.stop_picking
    } else {
        &l.pick_element
    }
}

pub fn set_browser_shown<R: Runtime>(handle: &AppHandle<R>, shown: bool) -> tauri::Result<()> {
    if let Some(state) = handle.try_state::<ViewState>() {
        state.browser_shown.store(shown, Ordering::Relaxed);
    }
    let l = Labels::in_language(crate::locale::language(), &handle.package_info().name);
    relabel(handle, BROWSER_ID, browser_label(&l, shown))
}

pub fn set_pick<R: Runtime>(handle: &AppHandle<R>, active: bool, enabled: bool) -> tauri::Result<()> {
    if let Some(state) = handle.try_state::<ViewState>() {
        state.pick_active.store(active, Ordering::Relaxed);
        state.pick_enabled.store(enabled, Ordering::Relaxed);
    }
    let l = Labels::in_language(crate::locale::language(), &handle.package_info().name);
    relabel(handle, PICK_ID, pick_label(&l, active))?;
    match view_item(handle, PICK_ID) {
        Some(item) => item.set_enabled(enabled),
        None => Ok(()),
    }
}

fn relabel<R: Runtime>(handle: &AppHandle<R>, id: &str, text: &str) -> tauri::Result<()> {
    match view_item(handle, id) {
        Some(item) => item.set_text(text),
        None => Ok(()),
    }
}

fn view_item<R: Runtime>(handle: &AppHandle<R>, id: &str) -> Option<MenuItem<R>> {
    let item = handle.menu()?.get(VIEW_ID)?.as_submenu()?.get(id)?;
    item.as_menuitem().cloned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    pub about: String,
    pub settings: String,
    pub services: String,
    pub hide: String,
    pub hide_others: String,
    pub show_all: String,
    pub quit: String,
    pub file: String,
    pub edit: String,
    pub undo: String,
    pub redo: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub select_all: String,
    pub view: String,
    pub hide_browser: String,
    pub show_browser: String,
    pub pick_element: String,
    pub stop_picking: String,
    pub fullscreen: String,
    pub window: String,
    pub minimize: String,
    pub zoom: String,
    pub close_window: String,
    pub help: String,
}

impl Labels {
    pub fn in_language(lang: &str, app: &str) -> Self {
        let t = |key: &str| crate::locale::t_in(lang, &format!("menu.{key}"), &[("app", app)]);
        Labels {
            about: t("about"),
            settings: t("settings"),
            services: t("services"),
            hide: t("hide"),
            hide_others: t("hideOthers"),
            show_all: t("showAll"),
            quit: t("quit"),
            file: t("file"),
            edit: t("edit"),
            undo: t("undo"),
            redo: t("redo"),
            cut: t("cut"),
            copy: t("copy"),
            paste: t("paste"),
            select_all: t("selectAll"),
            view: t("view"),
            hide_browser: t("hideBrowser"),
            show_browser: t("showBrowser"),
            pick_element: t("pickElement"),
            stop_picking: t("stopPicking"),
            fullscreen: t("fullscreen"),
            window: t("window"),
            minimize: t("minimize"),
            zoom: t("zoom"),
            close_window: t("closeWindow"),
            help: t("help"),
        }
    }
}

pub fn build<R: Runtime>(handle: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let name = handle.package_info().name.clone();
    let l = Labels::in_language(crate::locale::language(), &name);
    #[cfg(target_os = "macos")]
    let app_menu = Submenu::with_items(
        handle,
        &name,
        true,
        &[
            &MenuItem::with_id(handle, ABOUT_ID, &l.about, true, None::<&str>)?,
            &PredefinedMenuItem::separator(handle)?,
            &MenuItem::with_id(handle, SETTINGS_ID, &l.settings, true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::services(handle, Some(&l.services))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::hide(handle, Some(&l.hide))?,
            &PredefinedMenuItem::hide_others(handle, Some(&l.hide_others))?,
            &PredefinedMenuItem::show_all(handle, Some(&l.show_all))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::quit(handle, Some(&l.quit))?,
        ],
    )?;
    #[cfg(not(target_os = "macos"))]
    let app_menu = Submenu::with_items(
        handle,
        &name,
        true,
        &[
            &MenuItem::with_id(handle, ABOUT_ID, &l.about, true, None::<&str>)?,
            &MenuItem::with_id(handle, SETTINGS_ID, &l.settings, true, Some("Ctrl+,"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::quit(handle, Some(&l.quit))?,
        ],
    )?;
    let file = Submenu::with_items(
        handle,
        &l.file,
        true,
        &[&PredefinedMenuItem::close_window(handle, Some(&l.close_window))?],
    )?;
    let edit = Submenu::with_items(
        handle,
        &l.edit,
        true,
        &[
            &PredefinedMenuItem::undo(handle, Some(&l.undo))?,
            &PredefinedMenuItem::redo(handle, Some(&l.redo))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::cut(handle, Some(&l.cut))?,
            &PredefinedMenuItem::copy(handle, Some(&l.copy))?,
            &PredefinedMenuItem::paste(handle, Some(&l.paste))?,
            &PredefinedMenuItem::select_all(handle, Some(&l.select_all))?,
        ],
    )?;
    let state = handle.try_state::<ViewState>();
    let shown = state.as_ref().map(|s| s.browser_shown.load(Ordering::Relaxed)).unwrap_or(false);
    let picking = state.as_ref().map(|s| s.pick_active.load(Ordering::Relaxed)).unwrap_or(false);
    let can_pick = state.as_ref().map(|s| s.pick_enabled.load(Ordering::Relaxed)).unwrap_or(false);
    let view = Submenu::with_id_and_items(
        handle,
        VIEW_ID,
        &l.view,
        true,
        &[
            &MenuItem::with_id(handle, BROWSER_ID, browser_label(&l, shown), true, Some("Alt+CmdOrCtrl+B"))?,
            &MenuItem::with_id(handle, PICK_ID, pick_label(&l, picking), can_pick, Some("Shift+CmdOrCtrl+C"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::fullscreen(handle, Some(&l.fullscreen))?,
        ],
    )?;
    let window = Submenu::with_id_and_items(
        handle,
        WINDOW_SUBMENU_ID,
        &l.window,
        true,
        &[
            &PredefinedMenuItem::minimize(handle, Some(&l.minimize))?,
            &PredefinedMenuItem::maximize(handle, Some(&l.zoom))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::close_window(handle, Some(&l.close_window))?,
        ],
    )?;
    let help = Submenu::with_id_and_items(handle, HELP_SUBMENU_ID, &l.help, true, &[])?;
    Menu::with_items(handle, &[&app_menu, &file, &edit, &view, &window, &help])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_items_carry_the_product_name() {
        let l = Labels::in_language("en", "Sonata");
        assert_eq!([l.about, l.hide, l.quit], ["About Sonata", "Hide Sonata", "Quit Sonata"].map(String::from));
    }

    #[test]
    fn the_labels_come_in_every_language() {
        let de = Labels::in_language("de", "Sonata");
        assert_eq!(
            [de.about, de.hide, de.quit],
            ["Über Sonata", "Sonata ausblenden", "Sonata beenden"].map(String::from)
        );
        assert_eq!(Labels::in_language("fr", "Sonata").quit, "Quitter Sonata");
    }

    #[test]
    fn every_menu_follows_the_language() {
        let menus = |l: Labels| [l.file, l.edit, l.view, l.window, l.help];
        assert_eq!(menus(Labels::in_language("en", "Sonata")), ["File", "Edit", "View", "Window", "Help"]);
        assert_eq!(
            menus(Labels::in_language("es", "Sonata")),
            ["Archivo", "Edición", "Visualización", "Ventana", "Ayuda"]
        );
        assert_eq!(
            menus(Labels::in_language("fr", "Sonata")),
            ["Fichier", "Édition", "Présentation", "Fenêtre", "Aide"]
        );
        assert_eq!(
            menus(Labels::in_language("de", "Sonata")),
            ["Ablage", "Bearbeiten", "Darstellung", "Fenster", "Hilfe"]
        );
        let es = Labels::in_language("es", "Sonata");
        assert_eq!([es.undo, es.select_all, es.hide_others], ["Deshacer", "Seleccionar todo", "Ocultar otros"]);
        let fr = Labels::in_language("fr", "Sonata");
        assert_eq!([fr.paste, fr.close_window, fr.show_all], ["Coller", "Fermer la fenêtre", "Tout afficher"]);
        let de = Labels::in_language("de", "Sonata");
        assert_eq!([de.minimize, de.fullscreen, de.services], ["Im Dock ablegen", "Vollbild", "Dienste"]);
    }

    #[test]
    fn no_label_is_missing_in_any_language() {
        for lang in ["en", "es", "fr", "de"] {
            let l = Labels::in_language(lang, "Sonata");
            let all = [
                &l.about,
                &l.settings,
                &l.services,
                &l.hide,
                &l.hide_others,
                &l.show_all,
                &l.quit,
                &l.file,
                &l.edit,
                &l.undo,
                &l.redo,
                &l.cut,
                &l.copy,
                &l.paste,
                &l.select_all,
                &l.view,
                &l.hide_browser,
                &l.show_browser,
                &l.pick_element,
                &l.stop_picking,
                &l.fullscreen,
                &l.window,
                &l.minimize,
                &l.zoom,
                &l.close_window,
                &l.help,
            ];
            for label in all {
                assert!(!label.starts_with("menu.") && !label.contains('&'), "{lang}: {label}");
            }
        }
    }

    #[test]
    fn the_apps_own_items_open_a_page_in_the_window() {
        assert_eq!(ui_event_for(SETTINGS_ID), Some("open-settings"));
        assert_eq!(ui_event_for(ABOUT_ID), Some("open-about"));
        assert_eq!(ui_event_for(BROWSER_ID), Some("toggle-browser"));
        assert_eq!(ui_event_for(PICK_ID), Some("toggle-pick"));
        assert_eq!(ui_event_for("quit"), None);
    }

    #[test]
    fn the_view_menu_has_no_sidebar_item() {
        assert_eq!(ui_event_for("sidebar"), None);
    }

    #[test]
    fn the_view_menu_offers_to_show_the_browser_panel_or_to_hide_it() {
        let en = Labels::in_language("en", "Sonata");
        assert_eq!([browser_label(&en, false), browser_label(&en, true)], ["Show Browser", "Hide Browser"]);
        let de = Labels::in_language("de", "Sonata");
        assert_eq!([browser_label(&de, false), browser_label(&de, true)], ["Browser einblenden", "Browser ausblenden"]);
    }

    #[test]
    fn the_view_menu_offers_to_pick_an_element_or_to_stop() {
        let en = Labels::in_language("en", "Sonata");
        assert_eq!([pick_label(&en, false), pick_label(&en, true)], ["Pick Element on Page", "Stop Picking"]);
        let de = Labels::in_language("de", "Sonata");
        assert_eq!(
            [pick_label(&de, false), pick_label(&de, true)],
            ["Element auf der Seite auswählen", "Auswahl beenden"]
        );
    }
}
