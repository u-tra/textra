use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use std::collections::{HashMap, VecDeque};
use std::thread;
use chrono::Local;
use winapi::um::winuser::*;
use winapi::shared::minwindef::*;
use winapi::ctypes::c_int;
use std::{ptr, mem};
use std::process::Command;

use crate::{load_config, ParseError, Replacement, TextraConfig};

const MAX_TEXT_LENGTH: usize = 100;
const KEY_DELAY: u64 = 10;

pub struct AppState {
    config: Arc<Mutex<TextraConfig>>,
    current_text: Arc<Mutex<VecDeque<char>>>,
    last_key_time: Arc<Mutex<Instant>>,
    shift_pressed: Arc<AtomicBool>,
    ctrl_pressed: Arc<AtomicBool>,
    caps_lock_on: Arc<AtomicBool>,
    killswitch: Arc<AtomicBool>,
}

impl AppState {
    pub fn new() -> Result<Self, ParseError> {
        let config = load_config()?;

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            current_text: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_TEXT_LENGTH))),
            last_key_time: Arc::new(Mutex::new(Instant::now())),
            shift_pressed: Arc::new(AtomicBool::new(false)),
            ctrl_pressed: Arc::new(AtomicBool::new(false)),
            caps_lock_on: Arc::new(AtomicBool::new(false)),
            killswitch: Arc::new(AtomicBool::new(false)),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    KeyEvent(DWORD, WPARAM, LPARAM),
    ConfigReload,
    Quit,
}

pub fn main_loop(app_state: Arc<AppState>, receiver: &std::sync::mpsc::Receiver<Message>) -> anyhow::Result<()> {
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

pub fn listen_keyboard(sender: std::sync::mpsc::Sender<Message>) -> anyhow::Result<()> {
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

lazy_static::lazy_static! {
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
) -> anyhow::Result<()> {
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
                VK_CAPITAL => {
                    let current = app_state.caps_lock_on.load(Ordering::SeqCst);
                    app_state.caps_lock_on.store(!current, Ordering::SeqCst);
                }
                VK_BACK => {
                    app_state.current_text.lock().unwrap().pop_back();
                }
                _ => {
                    if app_state.ctrl_pressed.load(Ordering::SeqCst) {
                        // Handle Ctrl+V (paste) operation
                        if vk_code as i32 == 'V' as i32 {
                            // For simplicity, we're not actually pasting here.
                            // In a real implementation, you'd want to get the clipboard content
                            // and add it to current_text.
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
        let mut keyboard_state: [u8; 256] = std::mem::zeroed();
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
            if shift_pressed {
                SYMBOL_PAIRS.get(&c).cloned().or(Some(c))
            } else {
                Some(c)
            }
        } else {
            None
        }
    }
}

fn check_and_replace(app_state: &AppState, current_text: &mut VecDeque<char>) -> anyhow::Result<()> {
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
) -> anyhow::Result<()> {
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

    // Block the listener before simulating key presses
    GENERATING.store(true, Ordering::SeqCst);

    // Backspace the original text
    let backspace_count = original.chars().count();
    let backspaces = vec![VK_BACK; backspace_count];
    simulate_key_presses(&backspaces, KEY_DELAY);

    // Type the replacement
    let vk_codes = string_to_vk_codes(&final_replacement);
    simulate_key_presses(&vk_codes, KEY_DELAY);

    // Unblock the listener after simulating key presses
    GENERATING.store(false, Ordering::SeqCst);

    // Update current_text
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

fn process_code_replacement(language: &str, code: &str) -> anyhow::Result<String> {
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
                .arg(code)
                .output()?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        "rust" => {
            // For Rust, we'll need to create a temporary file, compile it, and run it
            use std::fs::File;
            use std::io::Write;
            use tempfile::Builder;

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

fn reload_config(app_state: Arc<AppState>) -> Result<(), ParseError> {
    let mut config = app_state.config.lock().unwrap();
    *config = load_config()?;
    Ok(())
}

fn simulate_key_presses(vk_codes: &[i32], key_delay: u64) {
    let batch_size = 20;
    let delay = Duration::from_millis(key_delay);
    let input_count = vk_codes.len() * 2;
    let mut inputs: Vec<winapi::um::winuser::INPUT> = Vec::with_capacity(input_count);

    for &vk in vk_codes {
        // Key press event
        inputs.push(winapi::um::winuser::INPUT {
            type_: INPUT_KEYBOARD,
            u: unsafe { mem::zeroed() },
        });
        unsafe {
            let ki = inputs.last_mut().unwrap().u.ki_mut();
            ki.wVk = vk as u16;
            ki.dwFlags = 0;
        }

        // Key release event
        inputs.push(winapi::um::winuser::INPUT {
            type_: INPUT_KEYBOARD,
            u: unsafe { mem::zeroed() },
        });
        unsafe {
            let ki = inputs.last_mut().unwrap().u.ki_mut();
            ki.wVk = vk as u16;
            ki.dwFlags = KEYEVENTF_KEYUP;
        }
    }

    // Send inputs in batches
    for chunk in inputs.chunks(batch_size) {
        unsafe {
            SendInput(
                chunk.len() as u32,
                chunk.as_ptr() as *mut winapi::um::winuser::INPUT,
                std::mem::size_of::<winapi::um::winuser::INPUT>() as c_int,
            );
        }
        // Add a small delay between batches for more natural input
        thread::sleep(delay);
    }
}

fn string_to_vk_codes(s: &str) -> Vec<i32> {
    s.chars().map(|c| char_to_vk_code(c)).collect()
}

fn char_to_vk_code(c: char) -> i32 {
    let vk_code = unsafe { VkKeyScanW(c as u16) as i32 };
    let low_byte = vk_code & 0xFF;
    let high_byte = (vk_code >> 8) & 0xFF;

    if high_byte & 1 != 0 {
        // Shift is required
        VK_SHIFT
    } else {
        low_byte
    }
}

// Error handling
#[derive(Debug)]
pub enum TextraError {
    ConfigError(ParseError),
    IoError(std::io::Error),
    ExecutionError(String),
}

impl std::fmt::Display for TextraError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TextraError::ConfigError(e) => write!(f, "Configuration error: {}", e),
            TextraError::IoError(e) => write!(f, "I/O error: {}", e),
            TextraError::ExecutionError(e) => write!(f, "Execution error: {}", e),
        }
    }
}

impl std::error::Error for TextraError {}

impl From<ParseError> for TextraError {
    fn from(error: ParseError) -> Self {
        TextraError::ConfigError(error)
    }
}

impl From<std::io::Error> for TextraError {
    fn from(error: std::io::Error) -> Self {
        TextraError::IoError(error)
    }
}

// Main function to tie everything together
pub fn run() -> anyhow::Result<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let app_state = Arc::new(AppState::new()?);

    // Start the config watcher in a separate thread
    let config_watcher_sender = sender.clone();
    let config_watcher_handle = std::thread::spawn(move || {
        if let Err(e) = watch_config(config_watcher_sender) {
            eprintln!("Error watching config: {}", e);
        }
    });

    // Start the keyboard listener in a separate thread
    let keyboard_listener_handle = std::thread::spawn(move || {
        if let Err(e) = listen_keyboard(sender) {
            eprintln!("Error in keyboard listener: {}", e);
        }
    });

    // Run the main loop
    main_loop(app_state, &receiver)?;

    // Clean up
    config_watcher_handle.join().unwrap();
    keyboard_listener_handle.join().unwrap();

    Ok(())
}

fn watch_config(sender: std::sync::mpsc::Sender<Message>) -> anyhow::Result<()> {
    use notify::{Watcher, RecursiveMode};
    use std::path::Path;

    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = notify::recommended_watcher(tx)?;

    watcher.watch(Path::new("config.toml"), RecursiveMode::NonRecursive)?;

    loop {
        match rx.recv() {
            Ok(_) => {
                // Config file changed, send reload message
                sender.send(Message::ConfigReload)?;
            },
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("config", &"[TextraConfig]")
            .field("current_text", &"[VecDeque<char>]")
            .field("last_key_time", &self.last_key_time)
            .field("shift_pressed", &self.shift_pressed)
            .field("ctrl_pressed", &self.ctrl_pressed)
            .field("caps_lock_on", &self.caps_lock_on)
            .field("killswitch", &self.killswitch)
            .finish()
    }
}