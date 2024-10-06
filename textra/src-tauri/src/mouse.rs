use std::sync::{Arc, Mutex, atomic::{AtomicI32, Ordering}};
use std::time::{Duration, Instant};
use winapi::um::winuser::{SetWindowsHookExW, CallNextHookEx, UnhookWindowsHookEx, HC_ACTION, WH_MOUSE_LL, MSLLHOOKSTRUCT, WM_LBUTTONDOWN};
use winapi::shared::minwindef::{WPARAM, LPARAM, LRESULT};
use winapi::ctypes::c_int;
use anyhow::Result;

use crate::state::AppState;

const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: i32 = 4;

#[derive(Debug, Clone, Copy)]
pub enum ClickType {
    Single,
    Double,
}

static mut GLOBAL_APP_STATE: Option<Arc<AppState>> = None;

unsafe extern "system" fn mouse_hook_proc(code: c_int, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if code == HC_ACTION {
        let mouse_struct: *const MSLLHOOKSTRUCT = std::mem::transmute(l_param);
        let x = (*mouse_struct).pt.x;
        let y = (*mouse_struct).pt.y;

        match w_param as u32 {
            WM_LBUTTONDOWN => {
                if let Some(app_state) = &GLOBAL_APP_STATE {
                    app_state.update_mouse_click(x, y);
                    
                }
            }
            _ => {}
        }
    }

    CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
}

pub fn listen_mouse() -> Result<()> {

    let hook = unsafe {
        SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(mouse_hook_proc),
            std::ptr::null_mut(),
            0,
        )
    };

    if hook.is_null() {
        return Err(anyhow::anyhow!("Failed to set mouse hook"));
    }

    // Message loop
    unsafe {
        let mut msg: winapi::um::winuser::MSG = std::mem::zeroed();
        while winapi::um::winuser::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            winapi::um::winuser::TranslateMessage(&msg);
            winapi::um::winuser::DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(hook);
    }

    Ok(())
}

pub fn get_mouse_position() -> (i32, i32) {
    unsafe {
        let mut pt: winapi::shared::windef::POINT = std::mem::zeroed();
        winapi::um::winuser::GetCursorPos(&mut pt);
        (pt.x, pt.y)
    }
}


