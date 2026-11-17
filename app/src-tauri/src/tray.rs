use std::sync::Mutex;

use sonata_core::engine::Engine;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::locale::{language, t_in};

pub const TRAY_ID: &str = "sonata";
const OPEN_ID: &str = "tray-open";
const PAUSE_ID: &str = "tray-pause";
const QUIT_ID: &str = "tray-quit";

pub struct TrayItems {
    pause: Mutex<MenuItem<Wry>>,
}

pub fn labels_in(lang: &str, app: &str, paused: bool) -> [String; 3] {
    let pause = if paused { "tray.resume" } else { "tray.pause" };
    [t_in(lang, "tray.open", &[("app", app)]), t_in(lang, pause, &[]), t_in(lang, "tray.quit", &[("app", app)])]
}

const ICON_ROWS: [&str; 36] = [
    "....................................",
    "....................................",
    "....................................",
    "....................................",
    "....................................",
    "....................................",
    "....................................",
    "..............37accca73.............",
    "............4cfffffffffd5...........",
    "...........7fffffffffffffa..........",
    "..........5ffffb75457bffff4.........",
    "..........dffc2.......1afe1.........",
    ".........3ffe1..........21..........",
    ".........5ffc.......................",
    ".........3fff71.....................",
    "..........cffffa62..................",
    "..........2dffffffc84...............",
    "...........17dfffffffd71............",
    "..............48dffffffe5...........",
    ".................26aeffff5..........",
    "....................16effd..........",
    "......................2eff3.........",
    ".......................cff4.........",
    "..........31..........2fff2.........",
    ".........8fd3........1cffd..........",
    ".........cfffa52...48efff5..........",
    ".........4ffffffefffffff8...........",
    "..........3cffffffffffd5............",
    "............49dffffea6..............",
    "................221.................",
    "....................................",
    "....................................",
    "....................................",
    "....................................",
    "....................................",
    "....................................",
];

pub fn icon() -> Image<'static> {
    let side = ICON_ROWS.len();
    let mut rgba = Vec::with_capacity(side * side * 4);
    for row in ICON_ROWS {
        for cell in row.chars() {
            let alpha = cell.to_digit(16).map_or(0, |level| level as u8 * 17);
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    Image::new_owned(rgba, side as u32, side as u32)
}

fn menu(app: &AppHandle, paused: bool) -> tauri::Result<(Menu<Wry>, MenuItem<Wry>)> {
    let name = app.package_info().name.clone();
    let [open, pause, quit] = labels_in(language(), &name, paused);
    let open = MenuItem::with_id(app, OPEN_ID, open, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, PAUSE_ID, pause, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, quit, true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &PredefinedMenuItem::separator(app)?, &pause, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    Ok((menu, pause))
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let paused = app.state::<Engine>().paused().unwrap_or(false);
    let (menu, pause) = menu(app, paused)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon())
        .icon_as_template(true)
        .tooltip(&app.package_info().name)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN_ID => show_window(app),
            PAUSE_ID => {
                let engine = app.state::<Engine>().inner().clone();
                let paused = engine.paused().unwrap_or(false);
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = engine.set_paused(!paused).await {
                        tracing::error!(error = %e, "could not pause or resume from the menu bar");
                    }
                });
            }
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(TrayItems { pause: Mutex::new(pause) });
    Ok(())
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    let Some(items) = app.try_state::<TrayItems>() else {
        tracing::warn!("no menu-bar item to relabel");
        return;
    };
    let label = labels_in(language(), &app.package_info().name, paused)[1].clone();
    let relabeled = items.pause.lock().unwrap().set_text(label);
    if let Err(e) = relabeled {
        tracing::warn!(error = %e, "could not relabel the menu-bar item");
    }
}

pub fn rebuild(app: &AppHandle) -> tauri::Result<()> {
    let Some(items) = app.try_state::<TrayItems>() else {
        tracing::warn!("no menu-bar item to rebuild");
        return Ok(());
    };
    let mut current = items.pause.lock().unwrap();
    let paused = app.state::<Engine>().paused().unwrap_or(false);
    let (menu, pause) = menu(app, paused)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
    }
    *current = pause;
    Ok(())
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_window(crate::browser::MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_labels_follow_the_language_and_the_pause_state() {
        let en = ["Open Sonata", "Pause all bots", "Quit Sonata"].map(String::from);
        assert_eq!(labels_in("en", "Sonata", false), en);
        assert_eq!(labels_in("es", "Sonata", true)[1], "Reanudar los bots");
        assert_eq!(labels_in("de", "Sonata", false)[2], "Sonata beenden");
    }

    #[test]
    fn the_icon_is_a_36_by_36_black_template_shaded_as_its_rows_say() {
        let image = icon();
        assert_eq!((image.width(), image.height()), (36, 36));
        let rgba = image.rgba();
        assert_eq!(rgba.len(), 36 * 36 * 4);
        assert!(rgba.chunks(4).all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0), "black only: macOS tints a template");
        for (i, px) in rgba.chunks(4).enumerate() {
            let (x, y) = (i % 36, i / 36);
            let cell = ICON_ROWS[y].as_bytes()[x] as char;
            let want = if cell == '.' { 0 } else { cell.to_digit(16).unwrap() as u8 * 17 };
            assert_eq!(px[3], want, "pixel ({x}, {y}) has the alpha its row gives");
        }
    }

    #[test]
    fn the_mark_has_soft_edges_and_clears_the_border() {
        let image = icon();
        let alpha = |x: usize, y: usize| image.rgba()[(y * 36 + x) * 4 + 3];
        for i in 0..36 {
            assert_eq!([alpha(i, 0), alpha(i, 35), alpha(0, i), alpha(35, i)], [0; 4], "the border is transparent");
        }
        let alphas: Vec<u8> = image.rgba().chunks(4).map(|px| px[3]).collect();
        assert!(alphas.contains(&255), "a solid mark");
        assert!(alphas.iter().any(|&a| a > 0 && a < 255), "anti-aliased: the curves are not stairs");
    }
}
