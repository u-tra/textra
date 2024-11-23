use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::time::{Duration, Instant};
use std::collections::{HashMap, VecDeque};
use std::thread;
use chrono::Local;
use winapi::um::{libloaderapi::GetModuleHandleW, winuser::*, wingdi::*};
use winapi::shared::{minwindef::*, windef::*};
use winapi::ctypes::c_int;
use std::{ptr, mem};
use std::process::Command;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use notify::{Watcher, RecursiveMode};
use std::path::Path;
use anyhow::Result;
use lazy_static::lazy_static;
use tempfile::Builder;

use crate::{load_config, view, watch_config, AppState, Replacement, TextraConfig, MAX_TEXT_LENGTH};

const KEY_DELAY: u64 = 2;

#[derive(Debug, Clone, Copy)]
pub enum Message {
    KeyEvent(DWORD, WPARAM, LPARAM),
    ConfigReload,
    Quit,
}

pub fn main_loop(app_state: Arc<AppState>, receiver: &std::sync::mpsc::Receiver<Message>) -> Result<()> {
    while let Ok(msg) = receiver.recv() {
        match msg {
            Message::KeyEvent(vk_code, w_param, l_param) => {
                if let Err(e) = handle_key_event(Arc::clone(&app_state), vk_code, w_param, l_param) {
                    eprintln!("Error handling key event: {}", e);
                }
            }
            Message::ConfigReload => {
                if let Err(e) = reload_config(Arc::clone(&app_state)) {
                    eprintln!("Error reloading config: {}", e);
                }
            }
            Message::Quit => break,
        }
    }
    Ok(())
}

lazy_static! {
    static ref SYMBOL_PAIRS: HashMap<char, char> = {
        let mut m = HashMap::new();
        m.insert(';', ':');
        m.insert(',', '<');
        m.insert('.', '>');
        m.insert('/', '?');
        m.insert('\'', '"');
        m.insert('[', '{');
        m.insert(']', '}');
        m.insert('\\', '|');
        m.insert('`', '~');
        m.insert('1', '!');
        m.insert('2', '@');
        m.insert('3', '#');
        m.insert('4', '$');
        m.insert('5', '%');
        m.insert('6', '^');
        m.insert('7', '&');
        m.insert('8', '*');
        m.insert('9', '(');
        m.insert('0', ')');
        m.insert('-', '_');
        m.insert('=', '+');
        m
    };
}

fn handle_key_event(
    app_state: Arc<AppState>,
    vk_code: DWORD,
    w_param: WPARAM,
    l_param: LPARAM,
) -> Result<()> {
    let now = Instant::now();

    match w_param as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let mut last_key_time = app_state.last_key_time.lock().unwrap();
            if now.duration_since(*last_key_time) > Duration::from_millis(1000) {
                app_state.current_text.lock().unwrap().clear();
            }
            *last_key_time = now;

            match vk_code as i32 {
                VK_ESCAPE => {
                    app_state.killswitch.store(true, Ordering::SeqCst);
                }
                VK_SHIFT | VK_LSHIFT | VK_RSHIFT => {
                    app_state.shift_pressed.store(true, Ordering::SeqCst);
                }
                VK_CONTROL | VK_LCONTROL | VK_RCONTROL => {
                    app_state.ctrl_pressed.store(true, Ordering::SeqCst);
                }
                VK_MENU | VK_LMENU | VK_RMENU => {
                    app_state.alt_pressed.store(true, Ordering::SeqCst);
                }
                VK_CAPITAL => {
                    let current = app_state.caps_lock_on.load(Ordering::SeqCst);
                    app_state.caps_lock_on.store(!current, Ordering::SeqCst);
                }
                VK_BACK => {
                    app_state.current_text.lock().unwrap().pop_back();
                }
                _ => {
                    if app_state.ctrl_pressed.load(Ordering::SeqCst) {
                        if vk_code as i32 == 'V' as i32 {
                            app_state.current_text.lock().unwrap().clear();
                        }
                    } else if let Some(c) = get_char_from_vk(
                        vk_code as i32,
                        app_state.shift_pressed.load(Ordering::SeqCst),
                        app_state.caps_lock_on.load(Ordering::SeqCst),
                    ) {
                        let mut current_text = app_state.current_text.lock().unwrap();
                        current_text.push_back(c);
                        if current_text.len() > MAX_TEXT_LENGTH {
                            current_text.pop_front();
                        }
                        check_and_replace(&app_state, &mut current_text)?;
                    }
                }
            }
        }
        WM_KEYUP | WM_SYSKEYUP => match vk_code as i32 {
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => {
                app_state.shift_pressed.store(false, Ordering::SeqCst);
            }
            VK_CONTROL | VK_LCONTROL | VK_RCONTROL => {
                app_state.ctrl_pressed.store(false, Ordering::SeqCst);
            }
            VK_MENU | VK_LMENU | VK_RMENU => {
                app_state.alt_pressed.store(false, Ordering::SeqCst);
            }
            VK_ESCAPE => {
                app_state.killswitch.store(false, Ordering::SeqCst);
            }
            _ => {}
        },
        _ => {}
    }

    Ok(())
}

fn get_char_from_vk(vk_code: i32, shift_pressed: bool, caps_lock_on: bool) -> Option<char> {
    unsafe {
        let mut keyboard_state: [u8; 256] = [0; 256];
        if shift_pressed {
            keyboard_state[VK_SHIFT as usize] = 0x80;
        }
        if caps_lock_on {
            keyboard_state[VK_CAPITAL as usize] = 0x01;
        }
        GetKeyboardState(keyboard_state.as_mut_ptr());

        let scan_code = MapVirtualKeyExW(vk_code as u32, MAPVK_VK_TO_VSC_EX, ptr::null_mut()) as u16;
        let mut char_buffer: [u16; 2] = [0; 2];

        let result = ToUnicodeEx(
            vk_code as u32,
            scan_code as u32,
            keyboard_state.as_ptr(),
            char_buffer.as_mut_ptr(),
            2,
            0,
            GetKeyboardLayout(0),
        );

        if result == 1 {
            let c = char::from_u32(char_buffer[0] as u32)?;
            if shift_pressed || caps_lock_on {
                SYMBOL_PAIRS.get(&c).cloned().or(Some(c))
            } else {
                Some(c)
            }
        } else {
            None
        }
    }
}

fn check_and_replace(app_state: &AppState, current_text: &mut VecDeque<char>) -> Result<()> {
    let immutable_current_text: String = current_text.iter().collect();
    let config = app_state.config.lock().unwrap();
    for rule in &config.rules {
        for trigger in &rule.triggers {
            if immutable_current_text.ends_with(trigger) {
                match &rule.replacement {
                    Replacement::Simple(text) => {
                        perform_replacement(
                            current_text,
                            trigger,
                            text,
                            true,
                            false,
                            app_state,
                        )?;
                    }
                    Replacement::Multiline(text) => {
                        perform_replacement(
                            current_text,
                            trigger,
                            text,
                            false,
                            false,
                            app_state,
                        )?;
                    }
                    Replacement::Code { language, content } => {
                        let replacement = process_code_replacement(language, content)?;
                        perform_replacement(
                            current_text,
                            trigger,
                            &replacement,
                            false,
                            true,
                            app_state,
                        )?;
                    }
                }
                return Ok(());
            }
        }
    }
    Ok(())
}

fn perform_replacement(
    current_text: &mut VecDeque<char>,
    original: &str,
    replacement: &str,
    propagate_case: bool,
    dynamic: bool,
    app_state: &AppState,
) -> Result<()> {
    let final_replacement = if dynamic {
        process_dynamic_replacement(replacement)
    } else if propagate_case {
        propagate_case_fn(original, replacement)
    } else {
        replacement.to_string()
    };

    if app_state.killswitch.load(Ordering::SeqCst) {
        return Ok(());
    }

    let backspace_count = original.chars().count();
    let backspaces: Vec<KeyPress> = vec![KeyPress { modifiers: vec![], key: VK_BACK as i32 }; backspace_count];
    simulate_key_presses(&backspaces, KEY_DELAY)?;

    let vk_codes = string_to_vk_codes(&final_replacement, app_state.shift_pressed.load(Ordering::SeqCst), app_state.caps_lock_on.load(Ordering::SeqCst));
    simulate_key_presses(&vk_codes, KEY_DELAY)?;

    for _ in 0..original.len() {
        current_text.pop_back();
    }
    for c in final_replacement.chars() {
        current_text.push_back(c);
        if current_text.len() > MAX_TEXT_LENGTH {
            current_text.pop_front();
        }
    }

    Ok(())
}

 

fn propagate_case_fn(original: &str, replacement: &str) -> String {
    if original.chars().all(|c| c.is_uppercase()) {
        replacement.to_uppercase()
    } else if original.chars().next().map_or(false, |c| c.is_uppercase()) {
        let mut chars = replacement.chars();
        match chars.next() {
            None => String::new(),
            Some(first_char) => first_char.to_uppercase().collect::<String>() + chars.as_str(),
        }
    } else {
        replacement.to_string()
    }
}

fn process_dynamic_replacement(replacement: &str) -> String {
    match replacement.to_lowercase().as_str() {
        "{{date}}" => Local::now().format("%Y-%m-%d").to_string(),
        "{{time}}" => Local::now().format("%H:%M:%S").to_string(),
        _ => replacement.to_string(),
    }
}

fn reload_config(app_state: Arc<AppState>) -> Result<()> {
    let mut config = app_state.config.lock().unwrap();
    *config = load_config()?;
    Ok(())
}

fn simulate_key_presses(vk_codes: &[KeyPress], key_delay: u64) -> Result<()> {
    let delay = Duration::from_millis(key_delay);

    for key_press in vk_codes {
        // Press all modifiers
        for &modifier in &key_press.modifiers {
            let mut input_down = winapi::um::winuser::INPUT {
                type_: INPUT_KEYBOARD,
                u: unsafe { mem::zeroed() },
            };
            unsafe {
                let ki = input_down.u.ki_mut();
                ki.wVk = modifier as u16;
                ki.dwFlags = 0;
            }
            unsafe {
                SendInput(
                    1,
                    &input_down as *const _ as *mut _,
                    std::mem::size_of::<winapi::um::winuser::INPUT>() as c_int,
                );
            }
            thread::sleep(delay);
        }

        // Press the main key
        let mut input_down = winapi::um::winuser::INPUT {
            type_: INPUT_KEYBOARD,
            u: unsafe { mem::zeroed() },
        };
        unsafe {
            let ki = input_down.u.ki_mut();
            ki.wVk = key_press.key as u16;
            ki.dwFlags = 0;
        }
        unsafe {
            SendInput(
                1,
                &input_down as *const _ as *mut _,
                std::mem::size_of::<winapi::um::winuser::INPUT>() as c_int,
            );
        }
        thread::sleep(delay);

        // Release the main key
        let mut input_up = winapi::um::winuser::INPUT {
            type_: INPUT_KEYBOARD,
            u: unsafe { mem::zeroed() },
        };
        unsafe {
            let ki = input_up.u.ki_mut();
            ki.wVk = key_press.key as u16;
            ki.dwFlags = KEYEVENTF_KEYUP;
        }
        unsafe {
            SendInput(
                1,
                &input_up as *const _ as *mut _,
                std::mem::size_of::<winapi::um::winuser::INPUT>() as c_int,
            );
        }
        thread::sleep(delay);

        // Release all modifiers in reverse order
        for &modifier in key_press.modifiers.iter().rev() {
            let mut input_up = winapi::um::winuser::INPUT {
                type_: INPUT_KEYBOARD,
                u: unsafe { mem::zeroed() },
            };
            unsafe {
                let ki = input_up.u.ki_mut();
                ki.wVk = modifier as u16;
                ki.dwFlags = KEYEVENTF_KEYUP;
            }
            unsafe {
                SendInput(
                    1,
                    &input_up as *const _ as *mut _,
                    std::mem::size_of::<winapi::um::winuser::INPUT>() as c_int,
                );
            }
            thread::sleep(delay);
        }
    }

    Ok(())
}

fn string_to_vk_codes(s: &str, shift_pressed: bool, caps_lock_on: bool) -> Vec<KeyPress> {
    s.chars().filter_map(|c| {
        let vk_scan = unsafe { VkKeyScanW(c as u16) };
        if vk_scan == -1 {
            return None;
        }

        let vk_code = (vk_scan & 0xFF) as i32;
        let shift_state = (vk_scan >> 8) & 0xFF;

        let mut modifiers = Vec::new();

        if shift_state & 1 != 0 {
            modifiers.push(VK_SHIFT as i32);
        }
        if shift_state & 2 != 0 {
            modifiers.push(VK_CONTROL as i32);
        }
        if shift_state & 4 != 0 {
            modifiers.push(VK_MENU as i32);
        }

        if shift_pressed || caps_lock_on {
            SYMBOL_PAIRS.get(&c).cloned().map(|symbol| KeyPress {
                modifiers: modifiers.clone(),
                key: symbol as i32,
            })
        } else {
            Some(KeyPress {
                modifiers,
                key: vk_code,
            })
        }
    }).collect()
}

pub fn run_hook() -> Result<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let app_state = Arc::new(AppState::new()?);

    let config_watcher_sender = sender.clone();
    let config_watcher_handle = std::thread::spawn(move || {
        if let Err(e) = watch_config(config_watcher_sender) {
            eprintln!("Error watching config: {}", e);
        }
    });

    let keyboard_listener_handle = std::thread::spawn(move || {
        if let Err(e) = listen_keyboard(sender) {
            eprintln!("Error in keyboard listener: {}", e);
        }
    });

    main_loop(app_state, &receiver)?;

    config_watcher_handle.join().unwrap();
    keyboard_listener_handle.join().unwrap();

    Ok(())
}










 
static mut GLOBAL_SENDER: Option<std::sync::mpsc::Sender<Message>> = None;
static GENERATING: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if code >= 0 && !GENERATING.load(Ordering::SeqCst) {
        let kb_struct = *(l_param as *const KBDLLHOOKSTRUCT);
        let vk_code = kb_struct.vkCode;

        if let Some(sender) = &GLOBAL_SENDER {
            let _ = sender.send(Message::KeyEvent(vk_code, w_param, l_param));
        }
    }

    CallNextHookEx(ptr::null_mut(), code, w_param, l_param)
}

pub fn listen_keyboard(sender: std::sync::mpsc::Sender<Message>) -> Result<()> {
    unsafe {
        GLOBAL_SENDER = Some(sender);
    }
    
    unsafe {
        let hook = SetWindowsHookExA(WH_KEYBOARD_LL, Some(keyboard_hook_proc), ptr::null_mut(), 0);
        if hook.is_null() {
            return Err(anyhow::anyhow!("Failed to set keyboard hook: {}", std::io::Error::last_os_error()));
        }
        let mut msg: MSG = mem::zeroed();
        while GetMessageA(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }
        UnhookWindowsHookEx(hook);
    }
    Ok(())
}
 
#[derive(Debug, Clone)]
struct KeyPress {
    modifiers: Vec<i32>, // e.g., VK_SHIFT, VK_CONTROL, VK_MENU
    key: i32,             // main key
}
 
fn process_code_replacement(language: &str, code: &str) -> Result<String> {
    match language.to_lowercase().as_str() {
        "python" => {
            let output = Command::new("python")
                .arg("-c")
                .arg(code)
                .output()?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        "javascript" => {
            let output = Command::new("node")
                .arg("-e")
                .arg(code).output()?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        "rust" => {
            use std::fs::File;
            use std::io::Write;

            let dir = Builder::new().prefix("rust_exec").tempdir()?;
            let file_path = dir.path().join("main.rs");
            let mut file = File::create(&file_path)?;
            writeln!(file, "fn main() {{")?;
            writeln!(file, "    {}", code)?;
            writeln!(file, "}}")?;
            file.flush()?;

            let output = Command::new("rustc")
                .arg(&file_path)
                .arg("-o")
                .arg(dir.path().join("output"))
                .output()?;

            if !output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stderr).to_string());
            }

            let output = Command::new(dir.path().join("output"))
                .output()?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        _ => Err(anyhow::anyhow!("Unsupported language: {}", language)),
    }
}
 