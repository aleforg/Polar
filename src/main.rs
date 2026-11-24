mod attachments;
mod blocks;
mod editor;
mod notebook;
mod notes;
mod palette;
mod performance;
mod store;

use gpui_kit::{
    AppContext, Bounds, KeyBinding, Menu, MenuItem, OsAction, TitlebarOptions, WindowBounds,
    WindowOptions,
    base::input::{Copy, Cut, Paste, SelectAll},
    component::{Theme, ThemeMode},
    point, px, size,
};
use notebook::{Notebook, actions::*};
use std::{
    env, io,
    path::{Path, PathBuf},
};

fn data_dir_override() -> Option<PathBuf> {
    env::var_os("POLAR_DATA_DIR")
        .or_else(|| env::var_os("RIGAQUIETA_DATA_DIR"))
        .or_else(|| env::var_os("OPENBEAR_DATA_DIR"))
        .map(PathBuf::from)
}

fn default_data_dir(application_support: &Path) -> io::Result<PathBuf> {
    let current = application_support.join("Polar");
    // Keep existing notes, attachments and the archive lock together.
    for name in ["Polar", "Rigaquieta", "OpenBear"] {
        let directory = application_support.join(name);
        if directory.try_exists()? {
            return Ok(directory);
        }
    }
    Ok(current)
}

fn main() {
    let directory = data_dir_override();
    #[cfg(debug_assertions)]
    if std::env::var_os("POLAR_SMOKE_EDITOR").is_some() {
        assert!(
            directory.is_some(),
            "Smoke test requires an isolated data directory"
        );
    }
    let directory = directory.unwrap_or_else(|| {
        let home = env::var_os("HOME").expect("A home directory or POLAR_DATA_DIR is required");
        default_data_dir(&PathBuf::from(home).join("Library/Application Support"))
            .expect("Could not locate the notes archive")
    });

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            cx.bind_keys([
                KeyBinding::new("cmd-n", NewNote, None),
                KeyBinding::new("cmd-f", Find, None),
                KeyBinding::new("cmd-shift-m", EditNote, None),
                KeyBinding::new("escape", Escape, Some("Notebook")),
                KeyBinding::new("up", PreviousNote, Some("Notebook")),
                KeyBinding::new("down", NextNote, Some("Notebook")),
                KeyBinding::new("cmd-shift-p", Pin, None),
                KeyBinding::new("cmd-shift-backspace", Trash, None),
                KeyBinding::new("cmd-shift-l", CyclePanes, None),
                KeyBinding::new("cmd-shift-e", Export, None),
                KeyBinding::new("cmd-w", Close, None),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-z", UndoNote, None),
                KeyBinding::new("cmd-shift-z", RedoNote, None),
                KeyBinding::new("cmd-b", Bold, None),
                KeyBinding::new("cmd-i", Italic, None),
                KeyBinding::new("cmd-u", Underline, None),
            ]);
            cx.set_menus([
                Menu::new("Polar").items([MenuItem::action("Esci da Polar", Quit)]),
                Menu::new("File").items([
                    MenuItem::action("Nuova nota", NewNote),
                    MenuItem::action("Esporta nota…", Export),
                    MenuItem::action("Chiudi finestra", Close),
                ]),
                Menu::new("Modifica").items([
                    MenuItem::action("Annulla", UndoNote),
                    MenuItem::action("Ripristina", RedoNote),
                    MenuItem::separator(),
                    MenuItem::os_action("Taglia", Cut, OsAction::Cut),
                    MenuItem::os_action("Copia", Copy, OsAction::Copy),
                    MenuItem::os_action("Incolla", Paste, OsAction::Paste),
                    MenuItem::os_action("Seleziona tutto", SelectAll, OsAction::SelectAll),
                    MenuItem::separator(),
                    MenuItem::action("Ricerca", Find),
                    MenuItem::action("Correggi selezione", CorrectSelection),
                    MenuItem::action("Organizza appunti", OrganizeNote),
                    MenuItem::action("Passa tra modifica e lettura", EditNote),
                ]),
                Menu::new("Formato").items([
                    MenuItem::action("Grassetto", Bold),
                    MenuItem::action("Corsivo", Italic),
                    MenuItem::action("Sottolineato", Underline),
                ]),
                Menu::new("Visualizza").items([MenuItem::action("Cambia pannelli", CyclePanes)]),
                Menu::new("Nota").items([
                    MenuItem::action("Fissa in alto", Pin),
                    MenuItem::action("Sposta nel Cestino o ripristina", Trash),
                ]),
            ]);
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            let window_bounds =
                if cfg!(debug_assertions) && std::env::var_os("POLAR_SMOKE_EDITOR").is_some() {
                    WindowBounds::Windowed(Bounds::centered(None, size(px(900.), px(520.)), cx))
                } else {
                    WindowBounds::Maximized(Bounds::centered(None, size(px(1260.), px(780.)), cx))
                };
            let (window_handle, notebook) = gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Polar".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(13.), px(18.))),
                    }),
                    app_owns_titlebar_drag: true,
                    window_bounds: Some(window_bounds),
                    window_min_size: Some(size(px(800.), px(480.))),
                    app_id: Some("com.alessioforgione.polar".into()),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Notebook::new(directory, window, cx)),
            )
            .expect("Polar window");
            #[cfg(debug_assertions)]
            if std::env::var_os("POLAR_SMOKE_EDITOR").is_some() {
                notebook::run_smoke_test(window_handle, notebook.clone(), cx);
            }
            let _ = window_handle;
            let notebook = notebook.downgrade();
            cx.on_action(move |_: &Quit, cx| {
                if notebook
                    .update(cx, |notebook, cx| notebook.flush(cx))
                    .unwrap_or(true)
                {
                    cx.quit();
                }
            });
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::default_data_dir;
    use std::fs;

    #[test]
    fn prefers_the_most_recent_existing_archive_directory() -> std::io::Result<()> {
        let support = tempfile::tempdir()?;
        let current = support.path().join("Polar");
        let rigaquieta = support.path().join("Rigaquieta");
        let openbear = support.path().join("OpenBear");

        assert_eq!(default_data_dir(support.path())?, current);
        fs::create_dir(&openbear)?;
        assert_eq!(default_data_dir(support.path())?, openbear);
        fs::create_dir(&rigaquieta)?;
        assert_eq!(default_data_dir(support.path())?, rigaquieta);
        fs::create_dir(&current)?;
        assert_eq!(default_data_dir(support.path())?, current);
        Ok(())
    }
}
