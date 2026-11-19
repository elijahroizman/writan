//! Making Writan the app that opens a file type.
//!
//! macOS lets an app set itself as the default handler (Launch Services), so this does it
//! directly. Windows doesn't: since Windows 8 only the user can choose, in Settings. There
//! Writan registers itself as a capable app for its file types and opens its page in
//! Default Apps, where each type is one click.

use serde::Serialize;
use tauri::{AppHandle, Runtime};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultStatus {
    ext: String,
    /// None when this system can't tell (Linux).
    is_default: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDefaults {
    /// Windows: Settings opened, and the user finishes there.
    opened_settings: bool,
}

/// The installer's name for each extension (Tauri uses the association's `name` as the
/// Windows file class).
#[cfg_attr(not(windows), allow(dead_code))]
fn file_class<R: Runtime>(app: &AppHandle<R>, ext: &str) -> Option<String> {
    app.config()
        .bundle
        .file_associations
        .as_ref()?
        .iter()
        .find_map(|assoc| {
            assoc
                .ext
                .iter()
                .any(|e| e.0.eq_ignore_ascii_case(ext))
                .then(|| assoc.name.clone().unwrap_or_else(|| ext.to_string()))
        })
}

#[tauri::command]
pub fn default_apps<R: Runtime>(app: AppHandle<R>, exts: Vec<String>) -> Vec<DefaultStatus> {
    exts.into_iter()
        .map(|ext| {
            let is_default = is_default(&app, &ext);
            DefaultStatus { ext, is_default }
        })
        .collect()
}

#[tauri::command]
pub fn set_default_apps<R: Runtime>(
    app: AppHandle<R>,
    exts: Vec<String>,
) -> Result<SetDefaults, String> {
    set_defaults(&app, &exts)
}

// ── macOS

#[cfg(target_os = "macos")]
fn is_default<R: Runtime>(app: &AppHandle<R>, ext: &str) -> Option<bool> {
    let current = mac::default_handler(ext)?;
    Some(current.eq_ignore_ascii_case(&app.config().identifier))
}

#[cfg(target_os = "macos")]
fn set_defaults<R: Runtime>(app: &AppHandle<R>, exts: &[String]) -> Result<SetDefaults, String> {
    let bundle_id = &app.config().identifier;
    let failed: Vec<&str> = exts
        .iter()
        .filter(|ext| !mac::set_default_handler(ext, bundle_id))
        .map(String::as_str)
        .collect();
    if !failed.is_empty() {
        return Err(format!(
            "macOS didn't make Writan the default for .{}.",
            failed.join(", .")
        ));
    }
    Ok(SetDefaults {
        opened_settings: false,
    })
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void, CStr, CString};

    type CFStringRef = *const c_void;
    const UTF8: u32 = 0x0800_0100;
    const ROLES_ALL: u32 = 0xFFFF_FFFF;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(value: *const c_void);
    }

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn UTTypeCreatePreferredIdentifierForTag(
            tag_class: CFStringRef,
            tag: CFStringRef,
            conforming_to: CFStringRef,
        ) -> CFStringRef;
        fn LSCopyDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            role: u32,
        ) -> CFStringRef;
        fn LSSetDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            role: u32,
            handler: CFStringRef,
        ) -> i32;
    }

    /// An owned CFString, released on drop.
    struct CfString(CFStringRef);

    impl CfString {
        fn new(value: &str) -> Option<Self> {
            let c = CString::new(value).ok()?;
            // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
            let s = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) };
            (!s.is_null()).then_some(CfString(s))
        }

        fn to_string(&self) -> Option<String> {
            let mut buffer = [0 as c_char; 512];
            // SAFETY: the buffer is writable for its full length.
            let ok = unsafe {
                CFStringGetCString(self.0, buffer.as_mut_ptr(), buffer.len() as isize, UTF8)
            };
            // SAFETY: CFStringGetCString NUL-terminates on success.
            (ok != 0).then(|| {
                unsafe { CStr::from_ptr(buffer.as_ptr()) }
                    .to_string_lossy()
                    .into_owned()
            })
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            // SAFETY: we own this reference (Create/Copy rule).
            unsafe { CFRelease(self.0) }
        }
    }

    /// The type identifier macOS uses for files with this extension (declared or dynamic).
    fn content_type(ext: &str) -> Option<CfString> {
        let class = CfString::new("public.filename-extension")?;
        let tag = CfString::new(ext)?;
        // SAFETY: both strings are valid; the result follows the Create rule.
        let uti =
            unsafe { UTTypeCreatePreferredIdentifierForTag(class.0, tag.0, std::ptr::null()) };
        (!uti.is_null()).then_some(CfString(uti))
    }

    pub fn default_handler(ext: &str) -> Option<String> {
        let uti = content_type(ext)?;
        // SAFETY: `uti` is valid; the result follows the Copy rule.
        let handler = unsafe { LSCopyDefaultRoleHandlerForContentType(uti.0, ROLES_ALL) };
        if handler.is_null() {
            return Some(String::new());
        }
        CfString(handler).to_string()
    }

    pub fn set_default_handler(ext: &str, bundle_id: &str) -> bool {
        let (Some(uti), Some(id)) = (content_type(ext), CfString::new(bundle_id)) else {
            return false;
        };
        // SAFETY: both strings are valid for the call.
        unsafe { LSSetDefaultRoleHandlerForContentType(uti.0, ROLES_ALL, id.0) == 0 }
    }
}

// ── Windows

#[cfg(windows)]
const CAPABILITIES: &str = r"Software\Writan\Capabilities";

#[cfg(windows)]
fn is_default<R: Runtime>(app: &AppHandle<R>, ext: &str) -> Option<bool> {
    use windows_registry::{CLASSES_ROOT, CURRENT_USER};
    let ours = file_class(app, ext)?;
    let dotted = format!(".{ext}");
    // The user's choice in Explorer wins; without one, the class registered for the extension.
    let chosen = CURRENT_USER
        .open(format!(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{dotted}\UserChoice"
        ))
        .and_then(|key| key.get_string("ProgId"))
        .ok()
        .or_else(|| {
            CLASSES_ROOT
                .open(&dotted)
                .and_then(|key| key.get_string(""))
                .ok()
        });
    Some(chosen.is_some_and(|class| class.eq_ignore_ascii_case(&ours)))
}

#[cfg(windows)]
fn set_defaults<R: Runtime>(app: &AppHandle<R>, _exts: &[String]) -> Result<SetDefaults, String> {
    use tauri_plugin_opener::OpenerExt;
    use windows_registry::CURRENT_USER;
    fn fail(e: impl std::fmt::Display) -> String {
        format!("Couldn't register Writan with Windows: {e}")
    }
    let caps = CURRENT_USER.create(CAPABILITIES).map_err(fail)?;
    caps.set_string("ApplicationName", "Writan").map_err(fail)?;
    caps.set_string(
        "ApplicationDescription",
        "Word, PowerPoint, PDF, Markdown and text files in one tabbed window",
    )
    .map_err(fail)?;
    // Every type Writan opens, so its Default Apps page lists them all. The user picks the
    // ones they asked for there.
    let assoc = CURRENT_USER
        .create(format!(r"{CAPABILITIES}\FileAssociations"))
        .map_err(fail)?;
    for association in app.config().bundle.file_associations.iter().flatten() {
        for ext in &association.ext {
            let class = association.name.clone().unwrap_or_else(|| ext.0.clone());
            assoc
                .set_string(format!(".{}", ext.0), &class)
                .map_err(fail)?;
        }
    }
    CURRENT_USER
        .create(r"Software\RegisteredApplications")
        .and_then(|key| key.set_string("Writan", CAPABILITIES))
        .map_err(fail)?;
    // Keep the installer's existing file classes (ProgIDs), so Windows' UserChoice
    // hashes remain valid. Only the displayed application registration changes.
    if let Ok(registered) = CURRENT_USER.create(r"Software\RegisteredApplications") {
        if registered.get_string("Workspace").ok().as_deref()
            == Some(r"Software\Workspace\Capabilities")
        {
            let _ = registered.remove_value("Workspace");
            let _ = CURRENT_USER.remove_tree(r"Software\Workspace\Capabilities");
        }
    }
    app.opener()
        .open_url(
            "ms-settings:defaultapps?registeredAppUser=Writan",
            None::<&str>,
        )
        .map_err(|e| format!("Couldn't open Default Apps: {e}"))?;
    Ok(SetDefaults {
        opened_settings: true,
    })
}

// ── Elsewhere (Linux dev builds)

#[cfg(not(any(target_os = "macos", windows)))]
fn is_default<R: Runtime>(_app: &AppHandle<R>, _ext: &str) -> Option<bool> {
    None
}

#[cfg(not(any(target_os = "macos", windows)))]
fn set_defaults<R: Runtime>(_app: &AppHandle<R>, _exts: &[String]) -> Result<SetDefaults, String> {
    Err("Setting default apps isn't supported on this system.".into())
}
