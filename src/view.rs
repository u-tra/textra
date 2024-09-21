// use super::*;
// use anyhow::Result;
// use std::ffi::OsStr;
// use std::mem;
// use std::os::windows::ffi::OsStrExt;
// use std::ptr;
// use std::sync::Arc;
// use winapi::shared::minwindef::*;
// use winapi::shared::windef::*;
// use winapi::um::wingdi::*;
// use winapi::um::winuser::*;
// use winapi::um::errhandlingapi::GetLastError;
// use winapi::um::libloaderapi::GetModuleHandleW;

// // Constants for window dimensions and colors
// const WINDOW_WIDTH: i32 = 300;
// const WINDOW_HEIGHT: i32 = 300;
// const TEXT_COLOR: COLORREF = 0x00FFFFFF; // White text in BGR
// const HIGHLIGHT_COLOR: COLORREF = 0x006B6BFF; // Light red for key states in BGR
// const SUGGESTION_COLOR: COLORREF = 0x0050AF4C; // Green for suggestions in BGR

// #[derive(Debug, Clone)]
// pub struct Suggestion {
//     pub text: String,
//     pub score: u32,
// }

// // Helper function to convert Rust string to wide string
// fn wide_string(s: &str) -> Vec<u16> {
//     OsStr::new(s).encode_wide().chain(Some(0)).collect()
// }

// // Create a transparent, topmost overlay window
// pub fn create_overlay_window(app_state: Arc<AppState>) -> Result<()> {
//     unsafe {
//         let instance = GetModuleHandleW(ptr::null());
//         let class_name = wide_string("TransparentOverlayClass");

//         let wc = WNDCLASSEXW {
//             cbSize: mem::size_of::<WNDCLASSEXW>() as UINT,
//             style: CS_HREDRAW | CS_VREDRAW,
//             lpfnWndProc: Some(overlay_window_proc),
//             cbClsExtra: 0,
//             cbWndExtra: 0,
//             hInstance: instance,
//             hIcon: ptr::null_mut(),
//             hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
//             hbrBackground: ptr::null_mut(),
//             lpszMenuName: ptr::null(),
//             lpszClassName: class_name.as_ptr(),
//             hIconSm: ptr::null_mut(),
//         };

//         if RegisterClassExW(&wc) == 0 {
//             let error = GetLastError();
//             eprintln!("Failed to register window class: {}", error);
//             return Err(anyhow::anyhow!("Failed to register window class: {}", error));
//         }

//         let screen_width = GetSystemMetrics(SM_CXSCREEN);
//         let x = screen_width - WINDOW_WIDTH - 50;
//         let y = 50;

//         let hwnd = CreateWindowExW(
//             WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
//             class_name.as_ptr(),
//             wide_string("Transparent Overlay").as_ptr(),
//             WS_POPUP | WS_VISIBLE,
//             x,
//             y,
//             WINDOW_WIDTH,
//             WINDOW_HEIGHT,
//             ptr::null_mut(),
//             ptr::null_mut(),
//             instance,
//             ptr::null_mut(),
//         );

//         if hwnd.is_null() {
//             let error = GetLastError();
//             eprintln!("Failed to create window: {}", error);
//             return Err(anyhow::anyhow!("Failed to create overlay window: {}", error));
//         }

//         // Make the window fully transparent
//         SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);

//         ShowWindow(hwnd, SW_SHOWNA);
//         UpdateWindow(hwnd);
//         app_state.set_overlay_hwnd(hwnd);
//     }

//     Ok(())
// }

// // Window procedure to handle painting and input
// unsafe extern "system" fn overlay_window_proc(
//     hwnd: HWND,
//     msg: UINT,
//     wparam: WPARAM,
//     lparam: LPARAM,
// ) -> LRESULT {
//     match msg {
//         WM_PAINT => {
//             let mut ps = mem::zeroed::<PAINTSTRUCT>();
//             let hdc = BeginPaint(hwnd, &mut ps);

//             let mut rect = mem::zeroed::<RECT>();
//             GetClientRect(hwnd, &mut rect);

//             let mem_dc = CreateCompatibleDC(hdc);
//             let bitmap = CreateCompatibleBitmap(hdc, rect.right, rect.bottom);
//             let old_bitmap = SelectObject(mem_dc, bitmap as HGDIOBJ);

//             // Fill the background with transparent color
//             let brush = CreateSolidBrush(RGB(0, 0, 0));
//             FillRect(mem_dc, &rect, brush);
//             DeleteObject(brush as HGDIOBJ);

//             SetBkMode(mem_dc, TRANSPARENT as c_int);

//             // Draw your content here (e.g., text, shapes)

//             // Update the layered window
//             let mut blend = BLENDFUNCTION {
//                 BlendOp: AC_SRC_OVER,
//                 BlendFlags: 0,

//                 SourceConstantAlpha: 255, // Adjust alpha transparency here
//                 AlphaFormat: AC_SRC_ALPHA,
//             };
//             let mut pt_zero = POINT { x: 0, y: 0 };
//             let mut size = SIZE {
//                 cx: rect.right - rect.left,
//                 cy: rect.bottom - rect.top,

//             };
//             UpdateLayeredWindow(
//                 hwnd,
//                 hdc,    
//                 &mut pt_zero as *mut POINT,
//                 &mut size as *mut SIZE,




//                 mem_dc,


//                 &mut pt_zero as *mut POINT,
//                 0,
//                 &mut blend as *mut BLENDFUNCTION,

//                 ULW_ALPHA, 

//             );



//             // Clean up
//             SelectObject(mem_dc, old_bitmap);
//             DeleteObject(bitmap as HGDIOBJ);
//             DeleteDC(mem_dc);

//             EndPaint(hwnd, &ps);
//             0
//         }
//         WM_DESTROY => {
//             PostQuitMessage(0);
//             0
//         }
//         _ => DefWindowProcW(hwnd, msg, wparam, lparam),
//     }
// }

// // Update the overlay window content in real-time
// pub fn update_overlay(app_state: Arc<AppState>) -> Result<()> {
//     let hwnd = app_state.get_overlay_hwnd();
//     if hwnd.is_null() {
//         return Ok(());
//     }

//     unsafe {
//         let hdc = GetDC(hwnd);
//         let mut rect = mem::zeroed::<RECT>();
//         GetClientRect(hwnd, &mut rect);

//         let mem_dc = CreateCompatibleDC(hdc);
//         let bitmap = CreateCompatibleBitmap(hdc, rect.right, rect.bottom);
//         let old_bitmap = SelectObject(mem_dc, bitmap as HGDIOBJ);

//         // Fill the background with transparent color
//         let brush = CreateSolidBrush(RGB(0, 0, 0));
//         FillRect(mem_dc, &rect, brush);
//         DeleteObject(brush as HGDIOBJ);

//         SetBkMode(mem_dc, TRANSPARENT as c_int);

//         // Draw dynamic content here
//         draw_dynamic_content(mem_dc, &rect, &app_state);

//         // Update the layered window
//         let mut blend = BLENDFUNCTION {
//             BlendOp: AC_SRC_OVER,
//             BlendFlags: 0,

//             SourceConstantAlpha: 255, // Adjust alpha transparency here
//             AlphaFormat: AC_SRC_ALPHA,
//         };
//         let mut pt_zero = POINT { x: 0, y: 0 };
//         let mut size = SIZE {
//             cx: rect.right - rect.left,
//             cy: rect.bottom - rect.top,

//         };
//         UpdateLayeredWindow(
//             hwnd,
//             hdc,
//             &mut pt_zero as *mut POINT,
//             &mut size as *mut SIZE,

//             mem_dc,
//             &mut pt_zero as *mut POINT,
//             0,
//             &mut blend as *mut BLENDFUNCTION,

//             ULW_ALPHA,
//         );

//         // Clean up
//         SelectObject(mem_dc, old_bitmap);
//         DeleteObject(bitmap as HGDIOBJ);
//         DeleteDC(mem_dc);
//         ReleaseDC(hwnd, hdc);
//     }

//     Ok(())
// }

// // Function to draw dynamic content
// unsafe fn draw_dynamic_content(hdc: HDC, rect: &RECT, app_state: &AppState) {
//     // Example: Draw current time
//     let time_str = format!("{}", Local::now().format("%H:%M:%S"));
//     draw_text(hdc, &time_str, 48, TEXT_COLOR, rect);

//     // Draw key indicators
//     let indicators = get_key_indicators(app_state);
//     if !indicators.is_empty() {
//         draw_text(hdc, &indicators, 24, HIGHLIGHT_COLOR, rect);
//     }

//     // Draw suggestions
//     let suggestions = app_state.get_suggestions();
//     draw_suggestions(hdc, &suggestions, rect);
// }

// // Helper function to draw text
// unsafe fn draw_text(hdc: HDC, text: &str, font_size: i32, color: COLORREF, rect: &RECT) {
//     let font = CreateFontW(
//         font_size,
//         0 ,
//         0,
//         0,
//         FW_NORMAL,

//         FALSE as u32,
//         FALSE as u32,
//         FALSE as u32,

//         DEFAULT_CHARSET,
//         OUT_DEFAULT_PRECIS,
//         CLIP_DEFAULT_PRECIS,
//         CLEARTYPE_QUALITY,
//         FF_DONTCARE | DEFAULT_PITCH,
//         wide_string("Segoe UI").as_ptr(),
//     );
//     let old_font = SelectObject(hdc, font as HGDIOBJ);
//     SetTextColor(hdc, color);

//     let mut text_rect = *rect;
//     DrawTextW(
//         hdc,
//         wide_string(text).as_ptr(),
//         -1,
//         &mut text_rect,
//         DT_CENTER | DT_VCENTER | DT_SINGLELINE,
//     );

//     SelectObject(hdc, old_font);
//     DeleteObject(font as HGDIOBJ);
// }

// // Helper function to get key indicators
// fn get_key_indicators(app_state: &AppState) -> String {
//     let mut indicators = Vec::new();
//     if app_state.get_ctrl_pressed() {
//         indicators.push("CTRL");
//     }
//     if app_state.get_shift_pressed() {
//         indicators.push("SHIFT");
//     }
//     if app_state.get_alt_pressed() {
//         indicators.push("ALT");
//     }
//     if app_state.get_caps_lock_on() {
//         indicators.push("CAPS");
//     }
//     indicators.join(" ")
// }

// // Helper function to draw suggestions
// unsafe fn draw_suggestions(hdc: HDC, suggestions: &[Suggestion], rect: &RECT) {
//     let mut suggestion_rect = RECT {
//         left: rect.left,
//         top: rect.top + 100, // Adjust as needed
//         right: rect.right,
//         bottom: rect.bottom,
//     };

//     for (i, suggestion) in suggestions.iter().enumerate() {
//         let text = format!("{}. {} ({})", i + 1, suggestion.text, suggestion.score);
//         draw_text(hdc, &text, 24, SUGGESTION_COLOR, &suggestion_rect);
//         suggestion_rect.top += 30; // Move down for the next suggestion
//     }
// }

// // Function to destroy the overlay window
// pub fn destroy_overlay_window(app_state: Arc<AppState>) -> Result<()> {
//     let hwnd = app_state.get_overlay_hwnd();
//     if hwnd.is_null() {
//         return Ok(());
//     }

//     unsafe {
//         DestroyWindow(hwnd);
//         app_state.set_overlay_hwnd(ptr::null_mut());
//     }

//     Ok(())
// }
