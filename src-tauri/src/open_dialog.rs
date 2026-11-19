//! ⌘O: one dialog that opens documents or a folder. The stock dialogs pick one or the other,
//! so each platform gets its own: a Mac open panel that can choose both, and on Windows and
//! Linux the file dialog with an extra "Open Folder" button that takes the folder on screen.

use tauri::{Runtime, WebviewWindow};

/// The chosen documents and folders, or nothing when cancelled. `extensions` limits the
/// documents shown (without dots).
#[tauri::command]
pub async fn pick_paths<R: Runtime>(
    window: WebviewWindow<R>,
    extensions: Vec<String>,
) -> Result<Vec<String>, String> {
    pick(window, extensions).await
}

#[cfg(target_os = "macos")]
async fn pick<R: Runtime>(
    window: WebviewWindow<R>,
    extensions: Vec<String>,
) -> Result<Vec<String>, String> {
    let (tx, answer) = channel();
    window
        .run_on_main_thread(move || {
            use objc2::MainThreadMarker;
            use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
            use objc2_foundation::{NSArray, NSString};

            let Some(mtm) = MainThreadMarker::new() else {
                let _ = tx.send(Vec::new());
                return;
            };
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(true);
            panel.setAllowsMultipleSelection(true);
            // Other files are greyed out; folders stay choosable.
            let types: Vec<_> = extensions.iter().map(|e| NSString::from_str(e)).collect();
            #[allow(deprecated)]
            panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
            let mut paths = Vec::new();
            if panel.runModal() == NSModalResponseOK {
                for url in panel.URLs().iter() {
                    if let Some(path) = url.path() {
                        paths.push(path.to_string());
                    }
                }
            }
            let _ = tx.send(paths);
        })
        .map_err(|e| e.to_string())?;
    answer.await
}

#[cfg(target_os = "linux")]
async fn pick<R: Runtime>(
    window: WebviewWindow<R>,
    extensions: Vec<String>,
) -> Result<Vec<String>, String> {
    let (tx, answer) = channel();
    let owner = window.clone();
    window
        .run_on_main_thread(move || {
            use gtk::prelude::*;
            use gtk::{FileChooserAction, FileChooserDialog, FileFilter, ResponseType};

            const OPEN_FOLDER: ResponseType = ResponseType::Other(1);
            let parent = owner.gtk_window().ok();
            let dialog = FileChooserDialog::with_buttons(
                Some("Open"),
                parent.as_ref(),
                FileChooserAction::Open,
                &[
                    ("Cancel", ResponseType::Cancel),
                    ("Open Folder", OPEN_FOLDER),
                    ("Open", ResponseType::Accept),
                ],
            );
            dialog.set_select_multiple(true);
            dialog.set_default_response(ResponseType::Accept);
            let filter = FileFilter::new();
            filter.set_name(Some("Documents"));
            for ext in &extensions {
                filter.add_pattern(&format!("*.{ext}"));
                filter.add_pattern(&format!("*.{}", ext.to_uppercase()));
            }
            dialog.add_filter(filter);
            let response = dialog.run();
            let paths: Vec<String> = if response == ResponseType::Accept {
                dialog
                    .filenames()
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            } else if response == OPEN_FOLDER {
                // A folder selected in the list, or else the one the dialog is showing.
                let selected = dialog.filename().filter(|p| p.is_dir());
                selected
                    .or_else(|| dialog.current_folder())
                    .map(|p| p.to_string_lossy().into_owned())
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            dialog.close();
            let _ = tx.send(paths);
        })
        .map_err(|e| e.to_string())?;
    answer.await
}

#[cfg(windows)]
async fn pick<R: Runtime>(
    window: WebviewWindow<R>,
    extensions: Vec<String>,
) -> Result<Vec<String>, String> {
    let owner = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    // The dialog runs its own message loop on a thread of its own.
    tauri::async_runtime::spawn_blocking(move || windows_dialog::pick(owner, &extensions))
        .await
        .map_err(|e| e.to_string())?
}

/// The dialog answers on the main thread; wait for it off the async runtime.
#[cfg(not(windows))]
fn channel() -> (
    std::sync::mpsc::Sender<Vec<String>>,
    impl std::future::Future<Output = Result<Vec<String>, String>>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let answer = async move {
        tauri::async_runtime::spawn_blocking(move || rx.recv().unwrap_or_default())
            .await
            .map_err(|e| e.to_string())
    };
    (tx, answer)
}

#[cfg(windows)]
mod windows_dialog {
    use std::cell::RefCell;
    use std::rc::Rc;

    use windows::core::{implement, w, Interface, Ref, Result, BOOL, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::System::SystemServices::SFGAO_FOLDER;
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileDialog, IFileDialogControlEvents, IFileDialogControlEvents_Impl,
        IFileDialogCustomize, IFileDialogEvents, IFileDialogEvents_Impl, IFileOpenDialog,
        IShellItem, FDE_OVERWRITE_RESPONSE, FDE_SHAREVIOLATION_RESPONSE, FOS_ALLOWMULTISELECT,
        FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
    };

    const OPEN_FOLDER: u32 = 1;

    /// Hears the Open Folder button and closes the dialog with the folder.
    #[implement(IFileDialogEvents, IFileDialogControlEvents)]
    struct Events {
        folder: Rc<RefCell<Option<String>>>,
    }

    impl IFileDialogEvents_Impl for Events_Impl {
        fn OnFileOk(&self, _: Ref<IFileDialog>) -> Result<()> {
            Ok(())
        }
        fn OnFolderChanging(&self, _: Ref<IFileDialog>, _: Ref<IShellItem>) -> Result<()> {
            Ok(())
        }
        fn OnFolderChange(&self, _: Ref<IFileDialog>) -> Result<()> {
            Ok(())
        }
        fn OnSelectionChange(&self, _: Ref<IFileDialog>) -> Result<()> {
            Ok(())
        }
        fn OnShareViolation(
            &self,
            _: Ref<IFileDialog>,
            _: Ref<IShellItem>,
        ) -> Result<FDE_SHAREVIOLATION_RESPONSE> {
            Ok(FDE_SHAREVIOLATION_RESPONSE(0))
        }
        fn OnTypeChange(&self, _: Ref<IFileDialog>) -> Result<()> {
            Ok(())
        }
        fn OnOverwrite(
            &self,
            _: Ref<IFileDialog>,
            _: Ref<IShellItem>,
        ) -> Result<FDE_OVERWRITE_RESPONSE> {
            Ok(FDE_OVERWRITE_RESPONSE(0))
        }
    }

    impl IFileDialogControlEvents_Impl for Events_Impl {
        fn OnItemSelected(&self, _: Ref<IFileDialogCustomize>, _: u32, _: u32) -> Result<()> {
            Ok(())
        }
        fn OnButtonClicked(&self, customize: Ref<IFileDialogCustomize>, id: u32) -> Result<()> {
            if id != OPEN_FOLDER {
                return Ok(());
            }
            let dialog: IFileDialog = customize.ok()?.cast()?;
            unsafe {
                // A folder selected in the list, or else the one the dialog is showing.
                let selected = dialog.GetCurrentSelection().ok().filter(|item| {
                    item.GetAttributes(SFGAO_FOLDER)
                        .map(|a| a == SFGAO_FOLDER)
                        .unwrap_or(false)
                });
                let item = match selected {
                    Some(item) => item,
                    None => dialog.GetFolder()?,
                };
                if let Some(path) = path_of(&item) {
                    *self.folder.borrow_mut() = Some(path);
                    dialog.Close(ERROR_CANCELLED.to_hresult())?;
                }
            }
            Ok(())
        }
        fn OnCheckButtonToggled(
            &self,
            _: Ref<IFileDialogCustomize>,
            _: u32,
            _: BOOL,
        ) -> Result<()> {
            Ok(())
        }
        fn OnControlActivating(&self, _: Ref<IFileDialogCustomize>, _: u32) -> Result<()> {
            Ok(())
        }
    }

    unsafe fn path_of(item: &IShellItem) -> Option<String> {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as _));
        path
    }

    pub fn pick(owner: isize, extensions: &[String]) -> std::result::Result<Vec<String>, String> {
        unsafe {
            // The dialog needs a single-threaded apartment; a pool thread already in another one can't show it.
            let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            if init.is_err() {
                return Err(init.message());
            }
            let result = show(HWND(owner as _), extensions).map_err(|e| e.message());
            CoUninitialize();
            result
        }
    }

    unsafe fn show(owner: HWND, extensions: &[String]) -> Result<Vec<String>> {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(
            dialog.GetOptions()? | FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM,
        )?;
        dialog.SetTitle(w!("Open"))?;

        let spec = HSTRING::from(
            extensions
                .iter()
                .map(|e| format!("*.{e}"))
                .collect::<Vec<_>>()
                .join(";"),
        );
        let filters = [
            COMDLG_FILTERSPEC {
                pszName: w!("Documents"),
                pszSpec: PCWSTR(spec.as_ptr()),
            },
            COMDLG_FILTERSPEC {
                pszName: w!("All files"),
                pszSpec: w!("*.*"),
            },
        ];
        dialog.SetFileTypes(&filters)?;

        let customize: IFileDialogCustomize = dialog.cast()?;
        customize.AddPushButton(OPEN_FOLDER, w!("Open Folder"))?;

        let folder = Rc::new(RefCell::new(None));
        let events: IFileDialogEvents = Events {
            folder: folder.clone(),
        }
        .into();
        let cookie = dialog.Advise(&events)?;
        let shown = dialog.Show(Some(owner));
        let _ = dialog.Unadvise(cookie);

        if let Some(path) = folder.borrow_mut().take() {
            return Ok(vec![path]);
        }
        if shown.is_err() {
            // Cancelled (or the dialog failed): nothing to open.
            return Ok(Vec::new());
        }
        let items = dialog.GetResults()?;
        let mut paths = Vec::new();
        for i in 0..items.GetCount()? {
            if let Some(path) = path_of(&items.GetItemAt(i)?) {
                paths.push(path);
            }
        }
        Ok(paths)
    }
}
