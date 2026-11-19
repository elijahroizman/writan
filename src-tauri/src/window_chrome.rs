//! Windows: the title bar, the menu bar and the app's tab bar in one color.
//!
//! Left alone, Windows draws the caption in its own color (an accent tint, in dark mode) and
//! muda paints the menu bar a fixed grey, over the app's tab bar: three tones stacked. The page
//! sends its chrome colors here; the caption takes them through DWM (Windows 11), and a window
//! subclass paints the menu bar with them before muda's dark painter sees the messages.

use std::sync::Mutex;

use tauri::{Runtime, Theme, WebviewWindow};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, DrawTextW, FillRect, GetWindowDC, MapWindowPoints, OffsetRect,
    RedrawWindow, ReleaseDC, SetBkMode, SetTextColor, DT_CENTER, DT_HIDEPREFIX, DT_SINGLELINE,
    DT_VCENTER, HDC, RDW_FRAME, RDW_INVALIDATE, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_HOTLIGHT, ODS_NOACCEL, ODS_SELECTED,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawMenuBar, GetClientRect, GetMenuBarInfo, GetMenuItemInfoW, GetWindowRect, HMENU,
    MENUBARINFO, MENUITEMINFOW, MIIM_STRING, OBJID_MENU, WM_NCACTIVATE, WM_NCPAINT,
};

const WM_UAHDRAWMENU: u32 = 0x0091;
const WM_UAHDRAWMENUITEM: u32 = 0x0092;
const SUBCLASS_ID: usize = 0x5753;

// Undocumented, as muda and Notepad++ use them.
#[repr(C)]
struct UahMenu {
    hmenu: HMENU,
    hdc: HDC,
    flags: u32,
}

#[repr(C)]
struct UahDrawMenuItem {
    dis: DRAWITEMSTRUCT,
    um: UahMenu,
    // UAHMENUITEM: the item's position, then metrics this doesn't read.
    position: i32,
}

#[derive(Clone, Copy)]
struct Colors {
    background: COLORREF,
    text: COLORREF,
    hot: COLORREF,
    disabled: COLORREF,
}

static COLORS: Mutex<Option<Colors>> = Mutex::new(None);

fn colors() -> Option<Colors> {
    *COLORS.lock().unwrap_or_else(|e| e.into_inner())
}

/// "#rrggbb" as a Win32 color (0x00bbggrr).
fn colorref(hex: &str) -> Option<COLORREF> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    Some(COLORREF(r | (g << 8) | (b << 16)))
}

/// The page's chrome colors and appearance. `theme` is None when the app follows the system.
pub fn set<R: Runtime>(
    window: WebviewWindow<R>,
    background: String,
    text: String,
    hot: String,
    disabled: String,
    theme: Option<String>,
) -> Result<(), String> {
    let parse = |s: &str| colorref(s).ok_or_else(|| format!("Not a color: {s}"));
    let next = Colors {
        background: parse(&background)?,
        text: parse(&text)?,
        hot: parse(&hot)?,
        disabled: parse(&disabled)?,
    };
    *COLORS.lock().unwrap_or_else(|e| e.into_inner()) = Some(next);

    // Dark or light caption buttons and popup menus, as the app is.
    let theme = match theme.as_deref() {
        Some("dark") => Some(Theme::Dark),
        Some("light") => Some(Theme::Light),
        _ => None,
    };
    let _ = window.set_theme(theme);

    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    window
        .run_on_main_thread(move || unsafe { repaint(HWND(hwnd as _), next) })
        .map_err(|e| e.to_string())
}

/// After the menu is replaced (new shortcuts), muda subclasses the window again, ahead of this
/// painter; put the painter back in front.
pub fn menu_replaced<R: Runtime>(window: &WebviewWindow<R>) {
    let Some(colors) = colors() else { return };
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    let _ = window.run_on_main_thread(move || unsafe { repaint(HWND(hwnd as _), colors) });
}

unsafe fn repaint(hwnd: HWND, colors: Colors) {
    // Windows 11 colors the caption; Windows 10 ignores these and keeps its own.
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_CAPTION_COLOR,
        &colors.background as *const _ as _,
        size_of::<COLORREF>() as u32,
    );
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_TEXT_COLOR,
        &colors.text as *const _ as _,
        size_of::<COLORREF>() as u32,
    );
    // Subclass procs run newest first: reinstalling puts this one ahead of muda's.
    let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
    let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0);
    let _ = DrawMenuBar(hwnd);
    let _ = RedrawWindow(Some(hwnd), None, None, RDW_FRAME | RDW_INVALIDATE);
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let Some(colors) = colors() else {
        return DefSubclassProc(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_UAHDRAWMENU => {
            let menu = &*(lparam.0 as *const UahMenu);
            let mut bar = menu_bar_rect(hwnd);
            bar.top -= 1;
            fill(menu.hdc, &bar, colors.background);
            LRESULT(0)
        }
        WM_UAHDRAWMENUITEM => {
            draw_item(&mut *(lparam.0 as *mut UahDrawMenuItem), colors);
            LRESULT(0)
        }
        WM_NCPAINT | WM_NCACTIVATE => {
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            // Windows draws a light line under the menu bar; paint it over.
            let mut client = RECT::default();
            let mut window = RECT::default();
            if GetClientRect(hwnd, &mut client).is_ok() && GetWindowRect(hwnd, &mut window).is_ok()
            {
                let mut corners = [
                    windows::Win32::Foundation::POINT {
                        x: client.left,
                        y: client.top,
                    },
                    windows::Win32::Foundation::POINT {
                        x: client.right,
                        y: client.bottom,
                    },
                ];
                MapWindowPoints(Some(hwnd), None, &mut corners);
                let mut line = RECT {
                    left: corners[0].x,
                    top: corners[0].y - 1,
                    right: corners[1].x,
                    bottom: corners[0].y,
                };
                let _ = OffsetRect(&mut line, -window.left, -window.top);
                let hdc = GetWindowDC(Some(hwnd));
                fill(hdc, &line, colors.background);
                ReleaseDC(Some(hwnd), hdc);
            }
            result
        }
        _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
}

/// The menu bar, in window coordinates.
unsafe fn menu_bar_rect(hwnd: HWND) -> RECT {
    let mut info = MENUBARINFO {
        cbSize: size_of::<MENUBARINFO>() as u32,
        ..Default::default()
    };
    let _ = GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut info);
    let mut window = RECT::default();
    let _ = GetWindowRect(hwnd, &mut window);
    let mut bar = info.rcBar;
    let _ = OffsetRect(&mut bar, -window.left, -window.top);
    bar
}

unsafe fn fill(hdc: HDC, rect: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    FillRect(hdc, rect, brush);
    let _ = DeleteObject(brush.into());
}

unsafe fn draw_item(item: &mut UahDrawMenuItem, colors: Colors) {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        ..Default::default()
    };
    if GetMenuItemInfoW(item.um.hmenu, item.position as u32, true, &mut info).is_err() {
        return;
    }
    let mut label = vec![0u16; info.cch as usize + 1];
    info.dwTypeData = windows::core::PWSTR(label.as_mut_ptr());
    info.cch += 1;
    if GetMenuItemInfoW(item.um.hmenu, item.position as u32, true, &mut info).is_err() {
        return;
    }
    label.truncate(info.cch as usize);

    let state = item.dis.itemState;
    let hot = state.0 & (ODS_HOTLIGHT.0 | ODS_SELECTED.0) != 0;
    let disabled = state.0 & (ODS_GRAYED.0 | ODS_DISABLED.0) != 0;
    let mut format = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
    if state.0 & ODS_NOACCEL.0 != 0 {
        format |= DT_HIDEPREFIX;
    }
    let hdc = item.um.hdc;
    fill(
        hdc,
        &item.dis.rcItem,
        if hot { colors.hot } else { colors.background },
    );
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(
        hdc,
        if disabled {
            colors.disabled
        } else {
            colors.text
        },
    );
    DrawTextW(hdc, &mut label, &mut item.dis.rcItem, format);
}
