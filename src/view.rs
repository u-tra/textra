use super::*;
use anyhow::Result;
use chrono::Local;
use winapi::um::errhandlingapi::GetLastError;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use std::time::Duration;
use std::{mem, ptr};
use winapi::shared::minwindef::*;
use winapi::shared::windef::*;
use winapi::um::dwmapi::*;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::wingdi::*;
use winapi::um::winuser::*;
use winapi::ctypes::c_int;



#[derive(Debug, Clone)]
pub struct Suggestion {
    pub text: String,
    pub score: u32,
}



// Constants for window dimensions and colors
const WINDOW_WIDTH: i32 = 300;
const WINDOW_HEIGHT: i32 = 300;
const TEXT_COLOR: u32 = 0xFFFFFFFF; // White text
const HIGHLIGHT_COLOR: u32 = 0xFFFF6B6B; // Light red for key states
const SUGGESTION_COLOR: u32 = 0xFF4CAF50; // Green for suggestions

// Create a transparent, borderless overlay window
pub fn create_overlay_window(app_state: Arc<AppState>) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let class_name = wide_string("TransparentOverlayClass");

        let wc = WNDCLASSEXW {
            cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: ptr::null_mut(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(), // Transparent background
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: ptr::null_mut(),
        };

        if RegisterClassExW(&wc) == 0 {
            let error = GetLastError();
            println!("Failed to register window class: {}", error);
            return Err(anyhow::anyhow!("Failed to register window class: {}", error));
        }

        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let x = screen_width - WINDOW_WIDTH - 50;
        let y = 50;

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
            class_name.as_ptr(),
            wide_string("Transparent Overlay").as_ptr(),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        );

        if hwnd.is_null() {
            let error = GetLastError();
            println!("Failed to create window: {}", error);
            return Err(anyhow::anyhow!("Failed to create overlay window: {}", error));
        }

        // Set the transparency for the overlay window
        if SetLayeredWindowAttributes(hwnd, 0, 200, LWA_ALPHA) == 0 {
            let error = GetLastError();
            println!("Failed to set layered window attributes: {}", error);
            return Err(anyhow::anyhow!("Failed to set layered window attributes: {}", error));
        }

        ShowWindow(hwnd, SW_SHOWNA);
        UpdateWindow(hwnd);
        app_state.set_overlay_hwnd(hwnd);
    }

    Ok(())
}

// Window procedure function to handle painting and input
unsafe extern "system" fn overlay_window_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect: RECT = mem::zeroed();
            GetClientRect(hwnd, &mut rect);

            let mem_dc = CreateCompatibleDC(hdc);
            let bitmap = CreateCompatibleBitmap(hdc, rect.right, rect.bottom);
            let old_bitmap = SelectObject(mem_dc, bitmap as *mut _);

            // Fill the background with a transparent color
            let brush = CreateSolidBrush(RGB(0, 0, 255));
            FillRect(mem_dc, &rect, brush);
            DeleteObject(brush as *mut _);

            // Draw the overlay text
            SetBkMode(mem_dc, TRANSPARENT as i32);
            SetTextColor(mem_dc, TEXT_COLOR);
            let text = wide_string("Transparent Overlay");
            DrawTextW(mem_dc, text.as_ptr(), -1, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

            // Layered window update with transparency
            let mut blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: 200,
                AlphaFormat: AC_SRC_ALPHA,
            };
            let mut point_src = POINT { x: 0, y: 0 };
            let mut size = SIZE { cx: rect.right, cy: rect.bottom };
            UpdateLayeredWindow(hwnd, hdc, ptr::null_mut(), &mut size, mem_dc, &mut point_src, 0, &mut blend, ULW_ALPHA);

            // Clean up
            SelectObject(mem_dc, old_bitmap);
            DeleteObject(bitmap as *mut _);
            DeleteDC(mem_dc);

            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// Update the overlay window with new content
pub fn update_overlay(app_state: Arc<AppState>) -> Result<()> {
    let hwnd = app_state.get_overlay_hwnd();
    if hwnd.is_null() {
        return Ok(());
    }

    unsafe {
        let hdc = GetDC(hwnd);
        let mut rect: RECT = mem::zeroed();
        GetClientRect(hwnd, &mut rect);

        let mem_dc = CreateCompatibleDC(hdc);
        let bitmap = CreateCompatibleBitmap(hdc, rect.right, rect.bottom);
        let old_bitmap = SelectObject(mem_dc, bitmap as *mut _);

        // Create a gradient brush for background
        let gradient_brush = create_radial_gradient_brush(mem_dc, &rect);
        FillRect(mem_dc, &rect, gradient_brush);
        DeleteObject(gradient_brush as *mut _);

        SetBkMode(mem_dc, TRANSPARENT as i32);

        // Draw the logo
        SetTextColor(mem_dc, TEXT_COLOR);
        draw_text(mem_dc, "TexTra", 48, &rect);

        // Draw key states
        let indicators = get_key_indicators(&app_state);
        SetTextColor(mem_dc, if indicators.is_empty() { TEXT_COLOR } else { HIGHLIGHT_COLOR });
        draw_text(mem_dc, &indicators, 24, &rect);

        // Draw current status text
        let current_text = app_state.get_current_status();
        draw_text(mem_dc, &current_text, 32, &rect);

        // Draw suggestions
        let suggestions = app_state.get_suggestions();
        draw_suggestions(mem_dc, &suggestions, &rect);

        // Update the layered window
        let mut blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA,
        };

        let mut point_src = POINT { x: 0, y: 0 };
        let mut size = SIZE { cx: rect.right - rect.left, cy: rect.bottom - rect.top };

        UpdateLayeredWindow(hwnd, hdc, ptr::null_mut(), &mut size, mem_dc, &mut point_src, 0, &mut blend, ULW_ALPHA);

        // Clean up
        SelectObject(mem_dc, old_bitmap);
        DeleteObject(bitmap as *mut _);
        DeleteDC(mem_dc);
        ReleaseDC(hwnd, hdc);
    }

    Ok(())
}

// Helper functions to convert Rust string to wide string
pub fn wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}


unsafe fn create_radial_gradient_brush(hdc: HDC, rect: &RECT) -> HBRUSH {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    let center_x = width / 2;
    let center_y = height / 2;

    let radius = ((center_x * center_x + center_y * center_y) as f32).sqrt() as i32;

    let vertex = [
        TRIVERTEX {
            x: center_x,
            y: center_y,
            Red: 0,
            Green: 0,
            Blue: 255 << 8, // Blue color
            Alpha: 255 << 8,
        },
        TRIVERTEX {
            x: rect.left,
            y: rect.top,
            Red: 255 << 8, // Red color
            Green: 0,
            Blue: 0,
            Alpha: 0,
        },
    ];

    let gradient_rect = [GRADIENT_RECT {
        UpperLeft: 0,
        LowerRight: 1,
    }];

    let mut brush : LOGBRUSH = mem::zeroed();
    brush.lbStyle = BS_SOLID;
    brush.lbColor = RGB(0, 0, 255);

    let h_brush = CreateBrushIndirect(&brush);

    winapi::um::wingdi::GradientFill(hdc, vertex.as_ptr() as *mut _, vertex.len() as u32, gradient_rect.as_ptr() as *mut _, gradient_rect.len() as u32, GRADIENT_FILL_RECT_V);

    h_brush
}


unsafe fn draw_text(hdc: HDC, text: &str, font_size: i32, rect: &RECT) {
    let font = CreateFontW(
        font_size,
        0,
        0,
        0,
        FW_NORMAL,
        0,
        0,
        0,
        ANSI_CHARSET,
        OUT_TT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        DEFAULT_PITCH | FF_DONTCARE,
        wide_string("Arial").as_ptr(),
    );

    let old_font = SelectObject(hdc, font as *mut _);
    let wide_text = wide_string(text);
    DrawTextW(
        hdc,
        wide_text.as_ptr(),
        -1,
        &mut rect.clone(),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    SelectObject(hdc, old_font);
    DeleteObject(font as *mut _);
}


fn get_key_indicators(app_state: &AppState) -> String {
    let mut indicators = String::new();
    if app_state.get_ctrl_pressed() {
        indicators.push_str("CTRL ");
    }
    if app_state.get_shift_pressed() {
        indicators.push_str("SHIFT ");
    }
    if app_state.get_alt_pressed() {
        indicators.push_str("ALT ");
    }
    if app_state.get_caps_lock_on() {
        indicators.push_str("CAPS ");
    }
    indicators
}


unsafe fn draw_suggestions(hdc: HDC, suggestions: &[Suggestion], rect: &RECT) {
    let mut suggestion_rect = RECT {
        left: rect.left,
        top: rect.top + 200, // Adjust to fit the layout
        right: rect.right,
        bottom: rect.bottom,
    };

    for (i, suggestion) in suggestions.iter().enumerate().take(3) {
        let suggestion_text = format!("{}. {} ({})", i + 1, suggestion.text, suggestion.score);
        draw_text(hdc, &suggestion_text, 24, &suggestion_rect);
        suggestion_rect.top += 30; // Adjust vertical spacing between suggestions
    }
}


pub fn destroy_overlay_window(app_state: Arc<AppState>) -> Result<()> {
    let hwnd = app_state.get_overlay_hwnd();
    if hwnd.is_null() {
        return Ok(());
    }

    unsafe {
        DestroyWindow(hwnd);
        app_state.set_overlay_hwnd(ptr::null_mut());
    }

    Ok(())
}

