//! The native menu bar. Every custom item emits a `menu` event carrying its id;
//! the frontend routes it to the active tab. Standard Edit items (undo, copy,
//! paste…) are predefined so they go straight to the focused text field.
//!
//! On Windows, WebView2 keeps keystrokes to itself, so menu accelerators never
//! fire while a document has focus. There the frontend handles every shortcut,
//! and the menu only shows the keys next to each item (Win32 right-aligns the
//! text after a tab).
//!
//! Shortcuts changed in Settings arrive through `set_menu_shortcuts`, which
//! rebuilds the menu with them.

use std::collections::HashMap;

use tauri::menu::{AboutMetadataBuilder, Menu, MenuItem, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

/// Menu item id → accelerator ("CmdOrCtrl+Shift+N"); an empty one removes the shortcut.
type Shortcuts = HashMap<String, String>;

fn item<R: Runtime>(
    app: &AppHandle<R>,
    keys: &Shortcuts,
    id: &str,
    label: &str,
    default: Option<&str>,
) -> tauri::Result<MenuItem<R>> {
    let accel = match keys.get(id) {
        Some(custom) if custom.is_empty() => None,
        Some(custom) => Some(custom.as_str()),
        None => default,
    };
    #[cfg(target_os = "macos")]
    {
        if let Some(accel) = accel {
            // A shortcut the menu can't parse leaves the item without one rather than losing the menu.
            if let Ok(built) = MenuItemBuilder::with_id(id, label)
                .accelerator(accel)
                .build(app)
            {
                return Ok(built);
            }
        }
        MenuItemBuilder::with_id(id, label).build(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let label = match accel {
            Some(accel) => format!("{label}\t{}", accel.replace("CmdOrCtrl", "Ctrl")),
            None => label.to_string(),
        };
        MenuItemBuilder::with_id(id, label).build(app)
    }
}

/// The menu with the default shortcuts, for startup.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    build_with(app, &Shortcuts::new())
}

/// Rebuild the menu bar with the shortcuts chosen in Settings.
#[tauri::command]
pub fn set_menu_shortcuts<R: Runtime>(
    app: AppHandle<R>,
    shortcuts: Shortcuts,
) -> Result<(), String> {
    let menu = build_with(&app, &shortcuts).map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if let Some(window) = tauri::Manager::get_webview_window(&app, "main") {
        crate::window_chrome::menu_replaced(&window);
    }
    Ok(())
}

fn build_with<R: Runtime>(app: &AppHandle<R>, keys: &Shortcuts) -> tauri::Result<Menu<R>> {
    let about = AboutMetadataBuilder::new()
        .name(Some("Writan"))
        .version(Some(env!("CARGO_PKG_VERSION")))
        .comments(Some(
            "Word, PowerPoint, PDF, Markdown and text files in one tabbed window.",
        ))
        .build();

    let reveal_label = if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(windows) {
        "Show in File Explorer"
    } else {
        "Show in Folder"
    };

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&item(app, keys, "new-tab", "New Tab", Some("CmdOrCtrl+T"))?)
        .item(&item(
            app,
            keys,
            "duplicate-tab",
            "Duplicate Tab",
            Some("CmdOrCtrl+Shift+D"),
        )?)
        // Asks which kind of file: one per plugin that's on.
        .item(&item(app, keys, "new", "New…", Some("CmdOrCtrl+N"))?)
        .item(&item(app, keys, "open", "Open…", Some("CmdOrCtrl+O"))?)
        // ⌘O opens folders too; this stays for finding by name.
        .item(&item(app, keys, "open-folder", "Open Folder…", None)?)
        .item(&item(app, keys, "close-folder", "Close Folder", None)?)
        .separator()
        .item(&item(app, keys, "save", "Save", Some("CmdOrCtrl+S"))?)
        .item(&item(
            app,
            keys,
            "save-as",
            "Save As…",
            Some("CmdOrCtrl+Shift+S"),
        )?)
        .item(&item(
            app,
            keys,
            "revert",
            "Reload from Disk",
            Some("CmdOrCtrl+R"),
        )?)
        .item(&item(
            app,
            keys,
            "version-history",
            "Version History…",
            None,
        )?)
        .separator()
        .item(&item(app, keys, "convert-file", "Convert File…", None)?)
        .item(&item(
            app,
            keys,
            "ocr-document",
            "Recognize Text (OCR)…",
            Some("CmdOrCtrl+Alt+O"),
        )?)
        .item(&item(
            app,
            keys,
            "copy-markdown",
            "Copy as Markdown (for LLMs)",
            None,
        )?)
        .item(&item(
            app,
            keys,
            "export-markdown",
            "Export as Markdown (.md)",
            None,
        )?)
        .item(&item(app, keys, "export-pdf", "Export as PDF", None)?)
        .item(&item(
            app,
            keys,
            "bundle-pdf",
            "Bundle Open Files to PDF…",
            None,
        )?)
        .separator()
        .item(&item(app, keys, "print", "Print…", Some("CmdOrCtrl+P"))?)
        .separator()
        .item(&item(app, keys, "reveal", reveal_label, None)?)
        .separator()
        .item(&item(
            app,
            keys,
            "close-tab",
            "Close Tab",
            Some("CmdOrCtrl+W"),
        )?)
        .item(&item(
            app,
            keys,
            "reopen-tab",
            "Reopen Closed Tab",
            Some("CmdOrCtrl+Shift+T"),
        )?);
    // Windows apps keep settings and Exit in the File menu; a Mac app has them in the app menu.
    #[cfg(not(target_os = "macos"))]
    let file_menu = file_menu
        .separator()
        .item(&item(
            app,
            keys,
            "settings",
            "Settings…",
            Some("CmdOrCtrl+,"),
        )?)
        .separator()
        .item(&item(app, keys, "quit", "Exit", Some("Alt+F4"))?);
    let file_menu = file_menu.build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .separator()
        .item(&item(app, keys, "find", "Find…", Some("CmdOrCtrl+F"))?)
        // Word's keys: Control-H on a Mac, where ⌘H hides the app.
        .item(&item(
            app,
            keys,
            "replace",
            "Find and Replace…",
            Some(if cfg!(target_os = "macos") {
                "Ctrl+H"
            } else {
                "CmdOrCtrl+H"
            }),
        )?)
        .build()?;

    let markdown_menu = SubmenuBuilder::new(app, "Source and Preview")
        .item(&item(
            app,
            keys,
            "view-read",
            "Preview Only",
            Some("CmdOrCtrl+Alt+1"),
        )?)
        .item(&item(
            app,
            keys,
            "view-split",
            "Source and Preview",
            Some("CmdOrCtrl+Alt+2"),
        )?)
        .item(&item(
            app,
            keys,
            "view-write",
            "Source Only",
            Some("CmdOrCtrl+Alt+3"),
        )?)
        .separator()
        .item(&item(
            app,
            keys,
            "view-cycle",
            "Next View",
            Some("CmdOrCtrl+E"),
        )?)
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&item(
            app,
            keys,
            "command-palette",
            "Command Palette…",
            Some("CmdOrCtrl+K"),
        )?)
        .separator()
        .item(&markdown_menu)
        .separator()
        .item(&item(
            app,
            keys,
            "toggle-explorer",
            "Toggle Folder",
            Some("CmdOrCtrl+Shift+E"),
        )?)
        .item(&item(
            app,
            keys,
            "split-view",
            "Split View",
            Some("CmdOrCtrl+Shift+\\"),
        )?)
        .item(&item(
            app,
            keys,
            "toggle-left",
            "Toggle Sidebar",
            Some("CmdOrCtrl+\\"),
        )?)
        .item(&item(
            app,
            keys,
            "toggle-right",
            "Toggle Inspector",
            Some("CmdOrCtrl+Alt+\\"),
        )?)
        .separator()
        .item(&item(app, keys, "zoom-in", "Zoom In", Some("CmdOrCtrl+="))?)
        .item(&item(
            app,
            keys,
            "zoom-out",
            "Zoom Out",
            Some("CmdOrCtrl+-"),
        )?)
        .item(&item(
            app,
            keys,
            "zoom-reset",
            "Actual Size",
            Some("CmdOrCtrl+0"),
        )?)
        .separator()
        .item(&item(
            app,
            keys,
            "focus-mode",
            "Focus Mode",
            Some("CmdOrCtrl+Alt+F"),
        )?)
        .item(&item(
            app,
            keys,
            "typewriter",
            "Typewriter Scrolling",
            Some("CmdOrCtrl+Alt+T"),
        )?)
        .item(&item(app, keys, "focus-dim", "Dim Other Paragraphs", None)?)
        .separator()
        .item(&item(
            app,
            keys,
            "toggle-theme",
            "Toggle Light/Dark",
            Some("CmdOrCtrl+Shift+L"),
        )?)
        .item(&item(
            app,
            keys,
            "change-theme",
            "Change Theme…",
            Some(if cfg!(target_os = "macos") {
                "Ctrl+Cmd+T"
            } else {
                "Ctrl+Alt+A"
            }),
        )?)
        .separator();
    #[cfg(target_os = "macos")]
    let view_menu = view_menu.fullscreen();
    #[cfg(not(target_os = "macos"))]
    let view_menu = view_menu.item(&item(
        app,
        keys,
        "toggle-fullscreen",
        "Full Screen",
        Some("F11"),
    )?);
    let view_menu = view_menu.build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .item(&item(
            app,
            keys,
            "next-tab",
            "Show Next Tab",
            Some(if cfg!(target_os = "macos") {
                "CmdOrCtrl+Shift+]"
            } else {
                "Ctrl+Tab"
            }),
        )?)
        .item(&item(
            app,
            keys,
            "prev-tab",
            "Show Previous Tab",
            Some(if cfg!(target_os = "macos") {
                "CmdOrCtrl+Shift+["
            } else {
                "Ctrl+Shift+Tab"
            }),
        )?)
        .build()?;

    let menus: Vec<Submenu<R>> = if cfg!(target_os = "macos") {
        let app_menu = SubmenuBuilder::new(app, "Writan")
            .about_with_text("About Writan", Some(about))
            .separator()
            .item(&item(
                app,
                keys,
                "settings",
                "Settings…",
                Some("CmdOrCtrl+,"),
            )?)
            .item(&item(
                app,
                keys,
                "check-updates",
                "Check for Updates…",
                None,
            )?)
            .item(&item(
                app,
                keys,
                "install-cli",
                "Install Command Line Tool…",
                None,
            )?)
            .separator()
            .services()
            .separator()
            .hide_with_text("Hide Writan")
            .hide_others()
            .show_all()
            .separator()
            // Custom quit so unsaved tabs get a chance to prompt.
            .item(&item(
                app,
                keys,
                "quit",
                "Quit Writan",
                Some("CmdOrCtrl+Q"),
            )?)
            .build()?;
        vec![app_menu, file_menu, edit_menu, view_menu, window_menu]
    } else {
        // No app menu: its Hide item would make the window vanish with no way back on Windows.
        let help_menu = SubmenuBuilder::new(app, "Help")
            .item(&item(
                app,
                keys,
                "check-updates",
                "Check for Updates…",
                None,
            )?)
            .separator()
            .about_with_text("About Writan", Some(about))
            .build()?;
        vec![file_menu, edit_menu, view_menu, window_menu, help_menu]
    };
    let refs: Vec<&dyn tauri::menu::IsMenuItem<R>> = menus
        .iter()
        .map(|m| m as &dyn tauri::menu::IsMenuItem<R>)
        .collect();
    Menu::with_items(app, &refs)
}
