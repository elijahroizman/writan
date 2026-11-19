//! Quitting from outside the app's own Quit command (the Dock's Quit, logging out, shutting down)
//! must ask about unsaved changes too. AppKit asks the app delegate whether it may terminate, but
//! tao's delegate doesn't answer, so the app quit at once and unsaved work was lost. Writan adds
//! the answer: it cancels native termination and runs the app's own Quit command, which checks
//! the current editor state and asks (as TextEdit does, this stops a logout until you answer).
//! Always go through the page: a cached dirty flag sent over IPC can lag behind the last edit.

#[cfg(target_os = "macos")]
pub fn install(app: &tauri::AppHandle) {
    use objc2::runtime::{AnyObject, Imp, Sel};
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    use std::sync::OnceLock;
    use tauri::Emitter;

    static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

    // NSApplicationTerminateReply: NSTerminateCancel is 0, NSTerminateNow 1.
    extern "C-unwind" fn should_terminate(
        _this: &AnyObject,
        _sel: Sel,
        _sender: *mut AnyObject,
    ) -> usize {
        if let Some(app) = APP.get() {
            let _ = app.emit("menu", "quit");
        }
        0
    }

    let _ = APP.set(app.clone());
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let ns_app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = ns_app.delegate() else {
        return;
    };
    let object: &AnyObject = (*delegate).as_ref();
    let class = object.class();
    if class
        .instance_method(sel!(applicationShouldTerminate:))
        .is_some()
    {
        return;
    }
    unsafe {
        let imp: Imp = std::mem::transmute(
            should_terminate as extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
        );
        // Returns NSUInteger (Q), takes self (@), _cmd (:) and the sender (@).
        objc2::ffi::class_addMethod(
            class as *const _ as *mut _,
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        );
    }
}
