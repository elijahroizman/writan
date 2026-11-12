mod cli;
mod defaults;
mod files;
mod fonts;
mod history;
mod icon;
// Helpers below are called from the macOS launch path. Other targets still
// compile them so the tests run, and nothing else references them.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod launch_services;
mod menu;
mod open_dialog;
mod plugins;
mod quit;
mod spelling;
mod tex;
mod update;
#[cfg(windows)]
mod window_chrome;

use std::sync::Mutex;

use tauri::{Emitter, Manager};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use tauri::RunEvent;

/// Paths the OS asked us to open (Finder "Open With", double-click, CLI args)
/// that the frontend hasn't picked up yet. The frontend drains this on startup
/// and whenever it receives an `open-requested` event, so nothing is lost if a
/// file arrives before the webview has finished loading.
#[derive(Default)]
pub struct PendingOpens(pub Mutex<Vec<String>>);

impl PendingOpens {
    fn push(&self, paths: impl IntoIterator<Item = String>) {
        self.0.lock().unwrap().extend(paths);
    }
}

#[tauri::command]
fn take_pending_opens(state: tauri::State<'_, PendingOpens>) -> Vec<String> {
    std::mem::take(&mut *state.0.lock().unwrap())
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    // The page has already asked about unsaved changes.
    app.exit(0);
}

fn queue_open(app: &tauri::AppHandle, paths: Vec<String>) {
    if !paths.is_empty() {
        app.state::<PendingOpens>().push(paths);
        let _ = app.emit("open-requested", ());
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// WebKit only underlines misspellings while typing when the app opts in (Safari does by
/// default; embedded web views don't). Set the default before the web view starts, unless
/// the user already chose otherwise from a Spelling menu.
#[cfg(target_os = "macos")]
fn enable_spell_checking() {
    use objc2_foundation::{NSString, NSUserDefaults};
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str("WebContinuousSpellCheckingEnabled");
    if defaults.objectForKey(&key).is_none() {
        defaults.setBool_forKey(true, &key);
    }
}

/// WebView2 runs browser shortcuts before the page can stop them: Ctrl+R and F5 reload the
/// app (losing unsaved work), Ctrl+F opens Edge's find bar. Turn those off; the page still gets
/// every key, and editing keys (copy, paste, undo) keep working.
#[cfg(windows)]
fn disable_browser_shortcuts(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows_core::Interface;
        let Ok(core) = webview.controller().CoreWebView2() else { return };
        let Ok(settings) = core.Settings() else { return };
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings3>() {
            let _ = settings.SetAreBrowserAcceleratorKeysEnabled(false);
        }
    });
}

/// The app's chrome colors, for the parts of the window Windows draws (title and menu bars).
/// Other systems draw no bars of their own over the page.
#[tauri::command]
fn set_window_chrome<R: tauri::Runtime>(window: tauri::WebviewWindow<R>, background: String, text: String, hot: String, disabled: String, theme: Option<String>) -> Result<(), String> {
    #[cfg(windows)]
    return window_chrome::set(window, background, text, hot, disabled, theme);
    #[cfg(not(windows))]
    {
        let _ = (window, background, text, hot, disabled, theme);
        Ok(())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    enable_spell_checking();
    // Captured before an update moves this bundle aside. See `relaunch_in_place`.
    #[cfg(target_os = "macos")]
    update::remember_bundle();

    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let cli_paths = cli::paths_from_args(&std::env::args().collect::<Vec<_>>(), &cwd);

    let builder = tauri::Builder::default();
    // Windows and Linux start a second copy for `writan notes.md` (or the older `workspace` command) or a double-click; it hands
    // its paths to this one and quits. (A Mac sends them to the running app itself.)
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
        queue_open(app, cli::paths_from_args(&args, &cwd));
    }));
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(PendingOpens(Mutex::new(cli_paths)))
        // Reading plugin files is disk I/O: answer off the thread that asked.
        .register_asynchronous_uri_scheme_protocol("wsplugin", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || responder.respond(plugins::serve(&app, &request)));
        })
        .setup(|app| {
            // A second Writan.app (an old install, a target/ build, a mounted
            // disk image) shows up in Open With as another version. Unregister
            // the others in the background. A dev run has no app bundle, so
            // there is nothing to register.
            #[cfg(target_os = "macos")]
            {
                let bundle_id = app.config().identifier.clone();
                let _ = std::thread::Builder::new()
                    .name("launch-services".into())
                    .spawn(move || launch_services::claim_single_open_with_entry(&bundle_id));
                quit::install(app.handle());
            }
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                disable_browser_shortcuts(&window);
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            let _ = app;
            Ok(())
        })
        .menu(menu::build)
        .on_menu_event(|app, event| {
            let _ = app.emit("menu", event.id().0.as_str());
        })
        .invoke_handler(tauri::generate_handler![
            files::read_file,
            files::write_file,
            files::path_info,
            files::list_dir,
            files::walk_dir,
            files::fetch_bytes,
            history::history_list,
            history::history_read,
            history::history_label,
            history::history_delete,
            history::history_usage,
            history::history_clear_all,
            take_pending_opens,
            cli::cli_status,
            cli::cli_install,
            cli::cli_uninstall,
            quit_app,
            update::latest_release,
            update::relaunch_in_place,
            update::linux_package,
            update::package_upgrade,
            menu::set_menu_shortcuts,
            open_dialog::pick_paths,
            spelling::spell_check,
            spelling::spell_suggest,
            spelling::spell_learn,
            set_window_chrome,
            icon::set_app_icon,
            plugins::installed_plugins,
            plugins::install_plugin,
            plugins::remove_plugin,
            tex::tex_status,
            tex::tex_compile,
            tex::tex_forget,
            tex::tex_install,
            tex::tex_uninstall_method,
            tex::tex_uninstall,
            fonts::installed_fonts,
            fonts::install_font,
            fonts::remove_font,
            fonts::font_file,
            defaults::default_apps,
            defaults::set_default_apps,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Writan");

    app.run(|app, event| match event {
        // Finder: double-click, "Open With", or dropping files on the Dock icon.
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        RunEvent::Opened { urls } => {
            let paths = urls
                .into_iter()
                .filter_map(|url| url.to_file_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            queue_open(app, paths);
        }
        // Clicking the Dock icon with no visible window brings the window back.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { has_visible_windows, .. } => {
            if !has_visible_windows {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }
        _ => {
            let _ = app;
        }
    });
}
