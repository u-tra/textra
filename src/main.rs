//! Textra - Text Expansion Utility
//! 
//! This application monitors keyboard input and replaces trigger text with expanded content
//! based on user-defined rules in a configuration file.

use std::{
    collections::{HashMap, VecDeque}, env, ffi::OsStr, fs, io::{self, Write}, mem, os::windows::{ffi::{OsStrExt, OsStringExt}, process::CommandExt}, path::{Path, PathBuf}, process::Command, ptr, sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    }, thread, time::{Duration, Instant}
};

use anyhow::{Context, Result, anyhow};
use chrono::Local;
use dirs;
use lazy_static::lazy_static;
use pest::Parser;
use pest_derive::Parser;
use pest::error::Error;
use pest::iterators::Pair;
use ropey::Rope;
use winapi::{
    shared::{
        minwindef::{DWORD, FALSE, LPARAM, LPVOID, WPARAM, UINT, LRESULT, HINSTANCE, BOOL},
        windef::{HWND, HBRUSH, POINT, RECT},
    },
    um::{
        errhandlingapi::GetLastError,
        fileapi::{CreateFileW, OPEN_EXISTING},
        handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
        libloaderapi::{GetModuleHandleW, GetProcAddress},
        minwinbase::{OVERLAPPED, SECURITY_ATTRIBUTES, STILL_ACTIVE},
        processthreadsapi::{GetExitCodeProcess, OpenProcess, TerminateProcess},
        synchapi::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
        tlhelp32::{CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS},
        winbase::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, INFINITE,
            WAIT_OBJECT_0, CREATE_NO_WINDOW, DETACHED_PROCESS,
        },
        winnt::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_LAST_WRITE,
            HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
        },
        winuser::{
            CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageA,
            DrawTextW, FillRect, GetAsyncKeyState, GetKeyboardLayout, GetKeyboardState,
            GetMessageA, GetSystemMetrics, LoadCursorW, MapVirtualKeyExW, RegisterClassExW,
            SendInput, SendMessageTimeoutA, SetLayeredWindowAttributes, SetWindowsHookExA,
            ShowWindow, ToUnicodeEx, TranslateMessage, UnhookWindowsHookEx, UpdateLayeredWindow,
            UpdateWindow, BeginPaint, EndPaint, GetKeyState, VkKeyScanW, GetClientRect,
            HWND_BROADCAST, IDC_ARROW, INPUT_KEYBOARD, KEYEVENTF_KEYUP, LWA_ALPHA, MAPVK_VK_TO_VSC_EX,
            SM_CXSCREEN, SW_SHOWNA, ULW_ALPHA, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_ESCAPE, VK_LCONTROL,
            VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_SHIFT, WH_KEYBOARD_LL,
            WM_KEYDOWN, WM_KEYUP, WM_SETTINGCHANGE, WM_SYSKEYDOWN, WM_SYSKEYUP, WS_EX_LAYERED, WS_EX_TOPMOST,
            WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE, CS_HREDRAW, CS_VREDRAW,
        },
    },
};
use winreg::{enums::*, RegKey};
use tempfile::Builder;
use minimo::{showln, whitebg, yellow_bold, cyan_bold, gray_dim, orange_bold, red_bold, green_bold, white_bold};

// ----- Common Types and Constants -----

const MAX_TEXT_LENGTH: usize = 100;
const SERVICE_NAME: &str = "textra";
const CONFIG_FILE_NAME: &str = "config.textra";
const KEY_DELAY: u64 = 2;

// ----- Error Types -----

#[derive(Debug)]
pub enum TextraError {
    IoError(io::Error),
    ParseError(ParseError),
    WindowsError(u32),
    ConfigError(String),
    Other(String),
}

impl From<io::Error> for TextraError {
    fn from(err: io::Error) -> Self {
        TextraError::IoError(err)
    }
}

impl From<ParseError> for TextraError {
    fn from(err: ParseError) -> Self {
        TextraError::ParseError(err)
    }
}

impl From<anyhow::Error> for TextraError {
    fn from(err: anyhow::Error) -> Self {
        TextraError::Other(err.to_string())
    }
}

impl std::fmt::Display for TextraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextraError::IoError(err) => write!(f, "IO Error: {}", err),
            TextraError::ParseError(err) => write!(f, "Parse Error: {}", err),
            TextraError::WindowsError(code) => write!(f, "Windows Error: {}", code),
            TextraError::ConfigError(msg) => write!(f, "Configuration Error: {}", msg),
            TextraError::Other(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl std::error::Error for TextraError {}

// ----- Message Types -----

#[derive(Debug, Clone, Copy)]
pub enum Message {
    KeyEvent(DWORD, WPARAM, LPARAM),
    ConfigReload,
    Quit,
}

// ----- Key Processing Types -----

#[derive(Debug, Clone)]
struct KeyPress {
    modifiers: Vec<i32>,
    key: i32,
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

// ----- Application State -----

pub struct AppState {
    pub config: Arc<Mutex<TextraConfig>>,
    pub current_text: Arc<Mutex<VecDeque<char>>>,
    pub last_key_time: Arc<Mutex<Instant>>,
    pub shift_pressed: Arc<AtomicBool>,
    pub ctrl_pressed: Arc<AtomicBool>,
    pub alt_pressed: Arc<AtomicBool>,
    pub caps_lock_on: Arc<AtomicBool>,
    pub killswitch: Arc<AtomicBool>,
}

impl AppState {
    pub fn new() -> Result<Self> {
        let config = load_config().context("Failed to load configuration")?;

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            current_text: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_TEXT_LENGTH))),
            last_key_time: Arc::new(Mutex::new(Instant::now())),
            shift_pressed: Arc::new(AtomicBool::new(false)),
            ctrl_pressed: Arc::new(AtomicBool::new(false)),
            alt_pressed: Arc::new(AtomicBool::new(false)),
            caps_lock_on: Arc::new(AtomicBool::new(false)),
            killswitch: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn get_current_status(&self) -> String {
        let current_text: String = self.current_text.lock().unwrap().iter().collect();
        format!(
            "Buffer: {}\nCtrl: {}\nShift: {}\nAlt: {}\nCaps Lock: {}",
            current_text,
            self.ctrl_pressed.load(Ordering::SeqCst),
            self.shift_pressed.load(Ordering::SeqCst),
            self.alt_pressed.load(Ordering::SeqCst),
            self.caps_lock_on.load(Ordering::SeqCst)
        )
    }
}

// ----- Configuration Types -----

#[derive(Debug, Clone)]
pub struct TextraConfig {
    pub metadata: HashMap<String, String>,
    pub documentation: Vec<String>,
    pub rules: Vec<TextraRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextraRule {
    pub triggers: Vec<String>,
    pub replacement: Replacement,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Replacement {
    Simple(String),
    Multiline(String),
    Code { language: String, content: String },
}

impl TextraConfig {
    fn score_replacement(&self, replacement: &Replacement, current_text: &str) -> f32 {
        match replacement {
            Replacement::Simple(s) => self.score_simple(s, current_text),
            Replacement::Multiline(s) => self.score_multiline(s, current_text),
            Replacement::Code { language, content } => self.score_code(language, content, current_text),
        }
    }

    fn score_simple(&self, s: &str, current_text: &str) -> f32 {
        let mut score = 0;
        let mut last_index = 0;
        for (i, c) in current_text.chars().enumerate() {
            if c == s.chars().next().unwrap_or_default() {
                score += 1;
                last_index = i;
            }
        }
        if current_text.len() <= last_index {
            return 0.0;
        }
        score as f32 / (current_text.len() - last_index) as f32
    }

    fn score_multiline(&self, s: &str, current_text: &str) -> f32 {
        let mut score = 0;
        let mut last_index = 0;
        for (i, c) in current_text.chars().enumerate() {
            if c == s.chars().next().unwrap_or_default() {
                score += 1;
                last_index = i;
            }
        }
        if current_text.len() <= last_index {
            return 0.0;
        }
        score as f32 / (current_text.len() - last_index) as f32
    }

    fn score_code(&self, language: &str, content: &str, current_text: &str) -> f32 {
        let mut score = 0;
        let mut last_index = 0;
        for (i, c) in current_text.chars().enumerate() {
            if c == content.chars().next().unwrap_or_default() {
                score += 1;
                last_index = i;
            }
        }
        if current_text.len() <= last_index {
            return 0.0;
        }
        score as f32 / (current_text.len() - last_index) as f32
    }
}

// ----- Configuration Parser -----

#[derive(Parser)]
#[grammar = "textra.pest"]
struct TextraParser;

pub type ParseError = pest::error::Error<Rule>;

pub fn parse_textra_config(input: &str) -> Result<TextraConfig, ParseError> {
    let mut config = TextraConfig {
        metadata: HashMap::new(),
        documentation: Vec::new(),
        rules: Vec::new(),
    };

    let pairs = TextraParser::parse(Rule::file, input)?;

    for pair in pairs {
        match pair.as_rule() {
            Rule::file => {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::metadata => parse_metadata(&mut config, inner_pair),
                        Rule::documentation => parse_documentation(&mut config, inner_pair),
                        Rule::rule => parse_rule(&mut config, inner_pair),
                        Rule::EOI => {}
                        _ => unreachable!(),
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    Ok(config)
}

fn parse_metadata(config: &mut TextraConfig, pair: Pair<Rule>) {
    let mut inner = pair.into_inner();
    let key = inner.next().unwrap().as_str().to_string();
    let value = inner.next().unwrap().as_str().to_string();
    config.metadata.insert(key, value);
}

fn parse_documentation(config: &mut TextraConfig, pair: Pair<Rule>) {
    let doc = pair.into_inner().next().unwrap().as_str().trim().to_string();
    config.documentation.push(doc);
}

fn parse_rule(config: &mut TextraConfig, pair: Pair<Rule>) {
    let mut inner = pair.into_inner();
    let triggers = parse_triggers(inner.next().unwrap());
    let replacement = parse_replacement(inner.next().unwrap());

    config.rules.push(TextraRule {
        triggers,
        replacement,
    });
}

fn parse_triggers(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .map(|trigger| trigger.as_str().trim().to_string())
        .collect()
}

fn parse_replacement(pair: Pair<Rule>) -> Replacement {
    match pair.as_rule() {
        Rule::replacement => {
            let inner = pair.into_inner().next().unwrap();
            match inner.as_rule() {
                Rule::simple_replacement => Replacement::Simple(inner.as_str().to_string()),
                Rule::multiline_replacement => {
                    let content = inner.into_inner().next().unwrap().as_str().to_string();
                    Replacement::Multiline(content)
                }
                Rule::code_replacement => {
                    let mut code_inner = inner.into_inner();
                    let language = code_inner.next().unwrap().as_str().trim().to_string();
                    let content = code_inner.next().unwrap().as_str().to_string();
                    Replacement::Code { language, content }
                }
                _ => unreachable!(),
            }
        }
        _ => unreachable!(),
    }
}

pub fn serialize_textra_config(config: &TextraConfig) -> String {
    let mut output = String::new();

    for (key, value) in &config.metadata {
        output.push_str(&format!("///{key}:{value}\n"));
    }

    for doc in &config.documentation {
        output.push_str(&format!("/// {doc}\n"));
    }

    for rule in &config.rules {
        let triggers = rule.triggers.join(" | ");
        let replacement = match &rule.replacement {
            Replacement::Simple(s) => s.to_string(),
            Replacement::Multiline(s) => format!("`{s}`"),
            Replacement::Code { language, content } => format!("```{language}\n{content}```"),
        };
        output.push_str(&format!("{triggers} => {replacement}\n"));
    }

    output
}

// ----- Configuration Management -----

pub fn load_config() -> Result<TextraConfig> {
    let config_path = get_config_path()?;
    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
    parse_textra_config(&config_str).context("Failed to parse configuration")
}

pub fn get_config_path() -> Result<PathBuf> {
    let home_dir = dirs::document_dir()
        .ok_or_else(|| anyhow!("Could not find documents directory"))?;
    let home_config_dir = home_dir.join("textra");
    let home_config_file = home_config_dir.join(CONFIG_FILE_NAME);

    if home_config_file.exists() {
        return Ok(home_config_file);
    }

    fs::create_dir_all(&home_config_dir)
        .context("Failed to create configuration directory")?;
    let home_config_file = home_config_dir.join(CONFIG_FILE_NAME);
    create_default_config(&home_config_file)
        .context("Failed to create default configuration")?;
    Ok(home_config_file)
}

pub fn create_default_config(path: &Path) -> Result<()> {
    fs::write(path, DEFAULT_CONFIG)
        .context("Failed to write default configuration file")?;
    Ok(())
}

pub fn watch_config(sender: Sender<Message>) -> Result<()> {
    let config_path = get_config_path()?;
    let config_dir = config_path.parent()
        .ok_or_else(|| anyhow!("Configuration path has no parent directory"))?;

    // Using Windows API to watch for file changes
    unsafe {
        let dir_handle = CreateFileW(
            to_wide_string(config_dir.to_str().unwrap_or_default()).as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            ptr::null_mut(),
        );

        if dir_handle == INVALID_HANDLE_VALUE {
            return Err(anyhow!("Failed to open directory for watching: {}", io::Error::last_os_error()));
        }

        let mut buffer = [0u8; 1024];
        let mut bytes_returned: DWORD = 0;
        let mut overlapped: OVERLAPPED = mem::zeroed();

        loop {
            let result = ReadDirectoryChangesW(
                dir_handle,
                buffer.as_mut_ptr() as LPVOID,
                buffer.len() as DWORD,
                FALSE,
                FILE_NOTIFY_CHANGE_LAST_WRITE,
                &mut bytes_returned,
                &mut overlapped,
                None,
            );

            if result == 0 {
                return Err(anyhow!("Failed to watch directory: {}", io::Error::last_os_error()));
            }

            let event = WaitForSingleObject(dir_handle, INFINITE);
            if event != WAIT_OBJECT_0 {
                return Err(anyhow!("Wait failed: {}", io::Error::last_os_error()));
            }

            // Send reload message when a change is detected
            if let Err(e) = sender.send(Message::ConfigReload) {
                return Err(anyhow!("Failed to send reload message: {}", e));
            }
        }
    }
}

pub fn reload_config(app_state: Arc<AppState>) -> Result<()> {
    let mut config = app_state.config.lock().unwrap();
    *config = load_config()?;
    Ok(())
}

// ----- Keyboard Input Handling -----

pub fn listen_keyboard(sender: Sender<Message>) -> Result<()> {
    unsafe {
        // We need to store the sender in a static variable for the callback
        KEYBOARD_SENDER = Some(sender);
        
        let hook = SetWindowsHookExA(
            WH_KEYBOARD_LL,
            Some(keyboard_hook_proc),
            ptr::null_mut(),
            0,
        );
        
        if hook.is_null() {
            return Err(anyhow!("Failed to set keyboard hook: {}", io::Error::last_os_error()));
        }
        
        // Message loop to keep the hook active
        let mut msg: winapi::um::winuser::MSG = mem::zeroed();
        while GetMessageA(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }
        
        UnhookWindowsHookEx(hook);
        Ok(())
    }
}

static mut KEYBOARD_SENDER: Option<Sender<Message>> = None;
static GENERATING: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if code >= 0 && !GENERATING.load(Ordering::SeqCst) {
        let kb_struct = *(l_param as *const winapi::um::winuser::KBDLLHOOKSTRUCT);
        let vk_code = kb_struct.vkCode;

        if let Some(sender) = &KEYBOARD_SENDER {
            let _ = sender.send(Message::KeyEvent(vk_code, w_param, l_param));
        }
    }

    CallNextHookEx(ptr::null_mut(), code, w_param, l_param)
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
                            // Clear buffer on paste to avoid partial triggering
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
        
        let scan_code = MapVirtualKeyExW(
            vk_code as u32,
            MAPVK_VK_TO_VSC_EX,
            ptr::null_mut()
        ) as u16;
        
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

    // Simulate backspaces to delete trigger text
    let backspace_count = original.chars().count();
    let backspaces: Vec<KeyPress> = vec![
        KeyPress { modifiers: vec![], key: VK_BACK as i32 };
        backspace_count
    ];
    simulate_key_presses(&backspaces, KEY_DELAY)?;

    // Simulate typing the replacement text
    let vk_codes = string_to_vk_codes(
        &final_replacement,
        app_state.shift_pressed.load(Ordering::SeqCst),
        app_state.caps_lock_on.load(Ordering::SeqCst),
    );
    simulate_key_presses(&vk_codes, KEY_DELAY)?;

    // Update the internal buffer
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

fn simulate_key_presses(vk_codes: &[KeyPress], key_delay: u64) -> Result<()> {
    let delay = Duration::from_millis(key_delay);
    GENERATING.store(true, Ordering::SeqCst);

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
                    std::mem::size_of::<winapi::um::winuser::INPUT>() as i32,
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
                std::mem::size_of::<winapi::um::winuser::INPUT>() as i32,
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
                std::mem::size_of::<winapi::um::winuser::INPUT>() as i32,
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
                    std::mem::size_of::<winapi::um::winuser::INPUT>() as i32,
                );
            }
            thread::sleep(delay);
        }
    }

    GENERATING.store(false, Ordering::SeqCst);
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

        Some(KeyPress {
            modifiers,
            key: vk_code,
        })
    }).collect()
}

fn process_code_replacement(language: &str, code: &str) -> Result<String> {
    match language.to_lowercase().as_str() {
        "python" => {
            let output = Command::new("python")
                .arg("-c")
                .arg(code)
                .output()
                .context("Failed to execute Python code")?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        "javascript" => {
            let output = Command::new("node")
                .arg("-e")
                .arg(code)
                .output()
                .context("Failed to execute JavaScript code")?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        "rust" => {
            use std::fs::File;
            use std::io::Write;

            let dir = Builder::new()
                .prefix("rust_exec")
                .tempdir()
                .context("Failed to create temporary directory")?;
            
            let file_path = dir.path().join("main.rs");
            let mut file = File::create(&file_path)
                .context("Failed to create Rust file")?;
            
            writeln!(file, "fn main() {{")?;
            writeln!(file, "    {}", code)?;
            writeln!(file, "}}")?;
            file.flush()?;

            let output = Command::new("rustc")
                .arg(&file_path)
                .arg("-o")
                .arg(dir.path().join("output"))
                .output()
                .context("Failed to compile Rust code")?;

            if !output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stderr).to_string());
            }

            let output = Command::new(dir.path().join("output"))
                .output()
                .context("Failed to execute compiled Rust code")?;
                
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        _ => Err(anyhow!("Unsupported language: {}", language)),
    }
}

// ----- Service Management -----

pub fn main_loop(app_state: Arc<AppState>, receiver: &Receiver<Message>) -> Result<()> {
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

pub fn handle_run() -> Result<()> {
    if is_service_running() {
        showln!(yellow_bold, "textra is already running.");
        return Ok(());
    }
    
    let exe_path = env::current_exe()
        .context("Failed to get current executable path")?;
        
    let mut command = Command::new(exe_path);
    command.arg("daemon");
    command.creation_flags(DETACHED_PROCESS);
    
    match command.spawn() {
        Ok(_) => {
            showln!(gray_dim, "textra service ", green_bold, "started.");
        }
        Err(e) => {
            return Err(anyhow!("Failed to start Textra service: {}", e));
        }
    }

    Ok(())
}

pub fn handle_daemon() -> Result<()> {
    let app_state = Arc::new(AppState::new()
        .context("Failed to create AppState")?);
    let (sender, receiver) = channel();

    let config_watcher = thread::spawn({
        let sender = sender.clone();
        move || watch_config(sender)
            .map_err(|e| anyhow!("Config watcher error: {}", e))
    });

    let keyboard_listener = thread::spawn({
        let sender = sender.clone();
        move || listen_keyboard(sender)
            .map_err(|e| anyhow!("Keyboard listener error: {}", e))
    });

    match main_loop(app_state, &receiver) {
        Ok(_) => {
            sender.send(Message::Quit).unwrap();
            config_watcher.join().unwrap()
                .context("Config watcher thread panicked")?;
            keyboard_listener.join().unwrap()
                .context("Keyboard listener thread panicked")?;
        }
        Err(e) => {
            sender.send(Message::Quit).unwrap();
            config_watcher.join().unwrap()
                .context("Config watcher thread panicked")?;
            keyboard_listener.join().unwrap()
                .context("Keyboard listener thread panicked")?;
            return Err(e);
        }
    }

    Ok(())
}

pub fn handle_stop() -> Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(anyhow!("Failed to create process snapshot"));
    }

    let mut entry: PROCESSENTRY32 = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32>() as u32;

    let mut found = false;

    unsafe {
        if Process32First(snapshot, &mut entry) != 0 {
            loop {
                let bytes = std::mem::transmute::<[i8; 260], [u8; 260]>(entry.szExeFile);
                let name = std::str::from_utf8_unchecked(
                    &bytes[..bytes.iter().position(|&x| x == 0).unwrap_or(260)],
                );

                if name.to_lowercase() == "textra.exe" {
                    found = true;
                    let process_handle = OpenProcess(PROCESS_TERMINATE, 0, entry.th32ProcessID);
                    if !process_handle.is_null() {
                        if TerminateProcess(process_handle, 0) != 0 {
                            showln!(gray_dim, "textra service ", red_bold, "stopped.");
                        } else {
                            showln!(orange_bold, "ooops! failed to stop textra service.");
                        }
                        CloseHandle(process_handle);
                    } else {
                        showln!(orange_bold, "ooops! failed to open textra process.");
                    }
                    break;
                }

                if Process32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }

    if !found {
        showln!(orange_bold, "textra service is not running.");
    }

    Ok(())
}

pub fn is_service_running() -> bool {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return false;
    }

    let mut entry: PROCESSENTRY32 = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32>() as u32;

    let mut textra_count = 0;
    let current_pid = std::process::id();

    unsafe {
        if Process32First(snapshot, &mut entry) != 0 {
            loop {
                let bytes = std::mem::transmute::<[i8; 260], [u8; 260]>(entry.szExeFile);
                let name = std::str::from_utf8_unchecked(
                    &bytes[..bytes.iter().position(|&x| x == 0).unwrap_or(260)],
                );

                if name.to_lowercase() == "textra.exe" && entry.th32ProcessID != current_pid as u32 {
                    let process_handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, entry.th32ProcessID);
                    if !process_handle.is_null() {
                        let mut exit_code: DWORD = 0;
                        if GetExitCodeProcess(process_handle, &mut exit_code) != 0 {
                            if exit_code == STILL_ACTIVE {
                                textra_count += 1;
                            }
                        }
                        CloseHandle(process_handle);
                    }
                }

                if Process32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }

    textra_count >= 1
}

// ----- Configuration UI Handling -----

pub fn handle_edit_config() -> Result<()> {
    let config_path = get_config_path()?;
    
    if let Ok(code_path) = which::which("code") {
        std::process::Command::new(code_path)
            .arg(&config_path)
            .spawn()
            .context("Failed to start VS Code")?;
    } else if let Ok(notepad_path) = which::which("notepad") {
        std::process::Command::new(notepad_path)
            .arg(&config_path)
            .spawn()
            .context("Failed to start Notepad")?;
    } else {
        return Err(anyhow!("No editor found. Please install Notepad or VS Code."));
    }
    
    Ok(())
}

pub fn display_config() {
    showln!(yellow_bold, "│ ", whitebg, " CONFIGURATION ");
    showln!(yellow_bold, "│ ");
    
    match load_config() {
        Ok(config) => {
            let config_path = get_config_path().unwrap();
            showln!(
                yellow_bold,
                "│ ",
                cyan_bold,
                "┌─ ",
                white_bold,
                config_path.display()
            );
            showln!(yellow_bold, "│ ", cyan_bold, "⇣ ");
            
            if !config.rules.is_empty() {
                for rule in &config.rules {
                    let (trigger, replace) = match &rule.replacement {
                        Replacement::Simple(text) => (&rule.triggers[0], text),
                        Replacement::Multiline(text) => (&rule.triggers[0], text),
                        Replacement::Code { language: _, content } => (&rule.triggers[0], content),
                    };
                    let trimmed = if replace.len() > 50 - trigger.len() {
                        format!("{}...", &replace[0..47 - trigger.len()]) 
                    } else {
                        replace.clone()
                    };

                    showln!(
                        yellow_bold,
                        "│ ",
                        cyan_bold,
                        "▫ ",
                        gray_dim,
                        trigger,
                        cyan_bold,
                        " ⋯→ ",
                        white_bold,
                        trimmed
                    );
                }
            }
        }
        Err(e) => {
            showln!(red_bold, "Error loading config: {}", e);
        }
    }
    
    showln!(yellow_bold, "│ ");
    showln!(
        yellow_bold,
        "└───────────────────────────────────────────────────────────────"
    );
    showln!(gray_dim, "");
}

// ----- Installation Management -----

pub fn get_install_dir() -> Result<PathBuf> {
    let d = dirs::home_dir()
        .map(|dir| dir.join(".textra"))
        .ok_or_else(|| anyhow!("Failed to determine home directory"))?;
        
    fs::create_dir_all(&d)
        .context("Failed to create installation directory")?;
        
    Ok(d)
}

pub fn auto_install() -> Result<()> {
    if !is_installed() {
        handle_install().context("Failed to install textra")?;
    }

    if !is_service_running() {
        handle_run().context("Failed to start daemon")?;
    };
    
    Ok(())
}

pub fn is_installed() -> bool {
    get_install_dir().map(|dir| dir.join("textra.exe").exists()).unwrap_or(false)
}

pub fn handle_install() -> Result<()> {
    showln!(gray_dim, "trying to install textra...");

    if is_service_running() {
        showln!(
            orange_bold,
            "an instance of textra is already running, stopping it..."
        );  
        handle_stop().context("Failed to stop running instance")?;
    }

    let exe_path = env::current_exe()
        .context("Failed to get current executable path")?;
        
    let install_dir = get_install_dir()?;
    let install_path = install_dir.join("textra.exe");
    
    showln!(
        gray_dim, 
        "copying ", 
        yellow_bold, 
        "textra.exe", 
        gray_dim, 
        " to ", 
        yellow_bold, 
        install_dir.to_string_lossy()
    );
    
    fs::copy(&exe_path, &install_path)
        .context("Failed to copy executable to install directory")?;

    add_to_path(&install_dir)
        .context("Failed to add Textra to PATH")?;
        
    set_autostart(&install_path)
        .context("Failed to set autostart")?;
        
    create_uninstaller(&install_dir)
        .context("Failed to create uninstaller")?;
        
    handle_run()
        .context("Failed to start service")?;
 
    Ok(())
}

pub fn is_running_from_install_dir() -> bool {
    if let Ok(exe_path) = env::current_exe() {
        if let Some(home_dir) = dirs::home_dir() {
            if exe_path.starts_with(&home_dir.join(".textra")) {
                return true;
            }
        }
    }
    false
}

pub fn handle_uninstall() -> Result<()> {
    showln!(gray_dim, "uninstalling textra from your system...");
   
    match handle_stop().context("Failed to stop running instance") {
        Ok(_) => {
            showln!(gray_dim, "textra service ", red_bold, "stopped.");
        }
        Err(e) => {
            showln!(
                orange_bold, 
                "oops! couldn't stop textra service. you can stop it manually by running uninstall.bat in .textra folder"
            );
        }
    }
    
    match remove_autostart().context("Failed to remove autostart entry") {
        Ok(_) => {
            showln!(gray_dim, "autostart entry removed.");
        }
        Err(e) => {
            showln!(gray_dim, "huh! couldn't remove autostart entry. maybe it's already removed.");
        }
    }

    match remove_from_path().context("Failed to remove textra from path") {
        Ok(_) => {
            showln!(gray_dim, "textra removed from path.");
        }
        Err(e) => {
            showln!(gray_dim, "couldn't find textra in path. skipping...");
        }
    }

    let install_dir = get_install_dir()?;
    match fs::remove_dir_all(&install_dir).context("Failed to remove installation directory") {
        Ok(_) => {
            showln!(gray_dim, "installation directory removed.");
        }
        Err(e) => {
            showln!(gray_dim, "couldn't remove installation directory. skipping...");
        }
    }

    showln!(gray_dim, "textra have been ", red_bold, "uninstalled", gray_dim, " from your system.");
    Ok(())
}

fn add_to_path(install_dir: &std::path::Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu
        .create_subkey("Environment")
        .context("Failed to open Environment registry key")?;

    let current_path: String = env
        .get_value("PATH")
        .context("Failed to get current PATH")?;
        
    if !current_path.contains(&install_dir.to_string_lossy().to_string()) {
        let new_path = format!("{};{}", current_path, install_dir.to_string_lossy());
        env.set_value("PATH", &new_path)
            .context("Failed to set new PATH")?;
            
        showln!(
            gray_dim,
            "added to ",
            green_bold,
            "path ",
            gray_dim,
            "environment variable."
        );
        
        showln!(
            gray_dim,
            "now you can access textra by typing",
            yellow_bold,
            " textra ",
            gray_dim,
            "in your terminal."
        );
    }

    update_environment_message();
 
    Ok(())
}

fn set_autostart(install_path: &std::path::Path) -> Result<()> {
    const AUTO_START_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(AUTO_START_PATH)
        .context("Failed to open Run registry key")?;
        
    let command = format!(
        r#"cmd /C start /min "" "{}" run"#,
        install_path.to_string_lossy()
    );
    
    key.set_value("Textra", &command)
        .context("Failed to set autostart registry value")?;

    showln!(
        gray_dim,
        "activated",
        green_bold,
        " automatic startup."
    );
    
    Ok(())
}

pub fn check_autostart() -> bool {
    const AUTO_START_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey(AUTO_START_PATH) {
        if let Ok(value) = key.get_value::<String, String>("Textra".to_string()) {
            if !value.is_empty() {
                return true;
            }
        }
    }
    false
}

fn remove_from_path() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu
        .create_subkey("Environment")
        .context("Failed to open Environment registry key")?;

    let current_path: String = env
        .get_value("PATH")
        .context("Failed to get current PATH")?;
        
    let install_dir = get_install_dir()?;
    let dir_str = install_dir.to_str()
        .ok_or_else(|| anyhow!("Installation directory path is not valid UTF-8"))?;
        
    let new_path: Vec<&str> = current_path
        .split(';')
        .filter(|&p| p != dir_str)
        .collect();
        
    let new_path = new_path.join(";");

    env.set_value("PATH", &new_path)
        .context("Failed to set new PATH")?;

    update_environment_message();

    showln!(
        gray_dim,
        "removed textra from path environment variable."
    );
    
    Ok(())
}

fn remove_autostart() -> Result<()> {
    const AUTO_START_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(AUTO_START_PATH)
        .context("Failed to open Run registry key")?;

    if let Err(e) = key.delete_value("Textra") {
        showln!(
            orange_bold,
            "Warning: Failed to remove autostart entry: {}",
            e
        );
    } else {
        showln!(
            gray_dim,
            "cancelling autostart..."
        );
    }

    Ok(())
}

fn create_uninstaller(install_dir: &std::path::Path) -> Result<()> {
    const UNINSTALLER_CODE: &str = r#"
    @echo off
    taskkill /F /IM textra.exe
    rmdir /S /Q "%LOCALAPPDATA%\Textra"
    reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v Textra /f
    echo Textra has been uninstalled.
"#;

    let uninstaller_path = install_dir.join("uninstall.bat");
    fs::write(&uninstaller_path, UNINSTALLER_CODE)
        .context("Failed to create uninstaller script")?;

    showln!(
        gray_dim,
        "textra have been ",
        green_bold, 
        "successfully installed ",
        gray_dim,
        "on this system."
    );

    showln!(
        gray_dim,
        "you can uninstall textra by running ",
        yellow_bold,
        "textra uninstall",
        gray_dim,
        " in the terminal"
    );
    
    Ok(())
}

fn update_environment_message() {
    unsafe {
        SendMessageTimeoutA(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            "Environment\0".as_ptr() as winapi::shared::minwindef::LPARAM,
            winapi::um::winuser::SMTO_ABORTIFHUNG,
            5000,
            ptr::null_mut(),
        );
    }
}

// ----- Update Management -----

pub fn handle_update() -> Result<()> {
    let latest_release = get_latest_release()?;
    let latest_version = parse_version_from_tag(&latest_release.tag_name)?;
    
    let textra_asset = latest_release.assets
        .iter()
        .find(|asset| asset.name == "textra.exe")
        .ok_or_else(|| anyhow!("Could not find textra.exe in release assets"))?;

    // Get paths
    let install_dir = get_install_dir()?;
    let current_exe = env::current_exe()?;
    let new_exe_path = install_dir.join("textra.new.exe");
    let update_script_path = install_dir.join("update.bat");

    // Download new version first
    showln!(gray_dim, "downloading version ", yellow_bold, &latest_version.to_string());
    download_file(&textra_asset.browser_download_url, &new_exe_path)?;

    // Create update batch script
    let batch_script = format!(
        r#"@echo off
setlocal enabledelayedexpansion

rem Wait a bit for parent process to exit
timeout /t 1 /nobreak >nul

rem Kill any running instances of textra
taskkill /F /IM textra.exe /T >nul 2>&1
timeout /t 1 /nobreak >nul

:RETRY_COPY
rem Try to copy new version over old version
copy /Y "{new}" "{current}" >nul 2>&1
if !errorlevel! neq 0 (
    timeout /t 1 /nobreak >nul
    goto RETRY_COPY
)

rem Start new version
start "" "{current}" run

rem Clean up
del "{new}" >nul 2>&1
del "%~f0"
"#,
        new = new_exe_path.display(),
        current = current_exe.display()
    );

    fs::write(&update_script_path, batch_script)?;

    // Launch the update script and exit
    showln!(gray_dim, "starting update process...");
    
    let status = Command::new("cmd")
       .args(&["/C", update_script_path.to_str().unwrap_or_default()])
       .current_dir(&install_dir)
       .creation_flags(CREATE_NO_WINDOW)
       .spawn()
       .context("Failed to start update process")?;

    showln!(gray_dim, "update prepared, restarting textra...");
    std::process::exit(0);
}

fn download_file(url: &str, path: &PathBuf) -> Result<()> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "Textra-Updater")
        .send()
        .context("Failed to download update")?;

    if response.status().is_success() {
        let content = response.bytes()
            .context("Failed to read download content")?;
        let mut file = std::fs::File::create(path)
            .context("Failed to create temporary file")?;
        file.write_all(&content)
            .context("Failed to write update to disk")?;
        Ok(())
    } else {
        Err(anyhow!("Download failed with status: {}", response.status()))
    }
}

#[derive(serde::Deserialize, Debug)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(serde::Deserialize, Debug)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    parts: Vec<u32>,
}

impl Version {
    fn parse(version_str: &str) -> Result<Self> {
        // Remove 'v' prefix if present
        let version_str = version_str.trim_start_matches('v');
        
        // Split and parse all parts as numbers
        let parts: Result<Vec<u32>, _> = version_str
            .split('.')
            .map(|s| s.parse::<u32>())
            .collect();

        let parts = parts.context(format!("Invalid version format: {}", version_str))?;
        Ok(Version { parts })
    }

    fn to_string(&self) -> String {
        self.parts.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(".")
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare each part, padding shorter version with 0s
        let max_len = self.parts.len().max(other.parts.len());
        for i in 0..max_len {
            let self_part = self.parts.get(i).copied().unwrap_or(0);
            let other_part = other.parts.get(i).copied().unwrap_or(0);
            
            match self_part.cmp(&other_part) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    }
}

fn get_latest_release() -> Result<GitHubRelease> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client")?;

    let response = client
        .get("https://api.github.com/repos/u-tra/textra/releases/latest")
        .header("User-Agent", "Textra-Updater")
        .send()
        .context("Failed to contact GitHub API")?;

    if response.status().is_success() {
        response.json::<GitHubRelease>()
            .context("Failed to parse GitHub response")
    } else {
        Err(anyhow!("GitHub API returned status: {}", response.status()))
    }
}

fn get_current_version() -> Result<Version> {
    let version_str = env!("CARGO_PKG_VERSION");
    Version::parse(version_str)
        .context("Failed to parse current version")
}

fn parse_version_from_tag(tag: &str) -> Result<Version> {
    Version::parse(tag)
        .context(format!("Failed to parse version from tag: {}", tag))
}

pub fn update_if_available() -> Result<()> {
    let current_version = get_current_version()?;
    showln!(
        gray_dim, 
        "checking for updates (current version: ", 
        yellow_bold, 
        &current_version.to_string(), 
        gray_dim, 
        ")"
    );

    match check_for_updates() {
        Ok(true) => {
            showln!(gray_dim, "new version available, preparing update...");
            handle_update()
        }
        Ok(false) => {
            showln!(gray_dim, "textra is up to date!");
            Ok(())
        }
        Err(e) => {
            showln!(orange_bold, "failed to check for updates: {}", e);
            Err(e)
        }
    }
}

pub fn check_for_updates() -> Result<bool> {
    let current_version = get_current_version()?;
    showln!(gray_dim, "current version: ", yellow_bold, &current_version.to_string());
    
    match get_latest_release() {
        Ok(latest_release) => {
            let latest_version = parse_version_from_tag(&latest_release.tag_name)?;
            showln!(gray_dim, "latest version: ", yellow_bold, &latest_version.to_string());
            
            Ok(latest_version > current_version)
        }
        Err(e) => {
            showln!(orange_bold, "failed to check for updates: {}", e);
            Ok(false) // Assume no update is available on error
        }
    }
}

// ----- CLI Interface -----

pub const BANNER: &str = r#"


  ██\                           ██\                        
  ██ |                          ██ |                       
██████\    ██████\  ██\   ██\ ██████\    ██████\  ██████\  
\_██  _|  ██  __██\ \██\ ██  |\_██  _|  ██  __██\ \____██\ 
  ██ |    ████████ | \████  /   ██ |    ██ |  \__|███████ |
  ██ |██\ ██   ____| ██  ██<    ██ |██\ ██ |     ██  __██ |
  \████  |\███████\ ██  /\██\   \████  |██ |     \███████ |
   \____/  \_______|\__/  \__|   \____/ \__|      \_______|
                                                
"#;

pub fn display_help() {
    BANNER.trim().lines().for_each(|line| showln!(white_bold,   line));
    showln!("");
    showln!(
        yellow_bold,
        "┌─ ",
        whitebg,
        " STATUS ",
        yellow_bold,
        " ──────────"
    );
    showln!(yellow_bold, "│ ");
    handle_display_status();
    showln!(yellow_bold, "│ ");
    showln!(yellow_bold, "│ ", whitebg, " HOW TO USE ");
    showln!(yellow_bold, "│ ");
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra run ",
        gray_dim,
        "- Start the Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra stop ",
        gray_dim,
        "- Stop the running Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra install ",
        gray_dim,
        "- Install Textra as a service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra uninstall ",
        gray_dim,
        "- Uninstall the Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra edit ",
        gray_dim,
        "- Edit the Textra configuration file"
    );
    showln!(yellow_bold, "│ ");

    display_config();
}

fn handle_display_status() {
    if is_service_running() {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "service: ",
            green_bold,
            "running."
        );
    } else {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "service: ",
            orange_bold,
            "not running."
        );
    }
    if check_autostart() {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "autostart: ",
            green_bold,
            "enabled."
        );
    } else {
        showln!(
            yellow_bold,
            "│ ",
            gray_dim,
            "autostart: ",
            orange_bold,
            "disabled."
        );
    }
}

// ----- Utility Functions -----

unsafe fn ReadDirectoryChangesW(
    handle: HANDLE,
    buffer: LPVOID,
    buffer_length: DWORD,
    watch_subtree: BOOL,
    notify_filter: DWORD,
    bytes_returned: *mut DWORD,
    overlapped: *mut OVERLAPPED,
    completion_routine: Option<fn()>,
) -> BOOL {
    type ReadDirectoryChangesWFn = unsafe extern "system" fn(
        HANDLE, LPVOID, DWORD, BOOL, DWORD, *mut DWORD, *mut OVERLAPPED, 
        Option<fn()>
    ) -> BOOL;
    
    let module = GetModuleHandleW(to_wide_string("kernel32.dll").as_ptr());
    let func_ptr = GetProcAddress(module, "ReadDirectoryChangesW\0".as_ptr() as *const i8);
    
    if let Some(func) = func_ptr.as_ref() {
        let func = std::mem::transmute::<_, ReadDirectoryChangesWFn>(func);
        func(handle, buffer, buffer_length, watch_subtree, notify_filter, 
             bytes_returned, overlapped, completion_routine)
    } else {
        0
    }
}

fn to_wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

const DEFAULT_CONFIG: &str = r#"
/// This is a Textra configuration file.
/// You can add your own triggers and replacements here.
/// When you type the text before `=>` it will be replaced with the text that follows.
/// It's as simple as that!

btw => by the way

:email => example@example.com

:psswd => 0nceUpon@TimeInPluto

pfa => please find the attached information as requested

pftb => please find the below information as required

:tst => `twinkle twinkle little star, how i wonder what you are,up above the world so high,like a diamond in the sky`

ccc => continue writing complete code without skipping anything

"#;

// ----- Main Entry Point -----

pub fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    
    //if applicaton is launched by double clicking the icon
    //we want window to stay open (usually it closes immediately)
    if args.len() == 1 {
        display_help();
        //wait for 2 seconds before closing
        std::thread::sleep(std::time::Duration::from_secs(2));
        return Ok(());
    }

    match args[1].as_str() {
        "run" | "start" => handle_run(),
        "config" | "edit" | "settings" => {
            handle_edit_config()?;
            Ok(())
        }
        "daemon" | "service" => handle_daemon(),
        "stop" | "kill" => handle_stop(),
        "install" | "setup" => handle_install(),
        "uninstall" | "remove" => handle_uninstall(),
        "update" => update_if_available(),
        _ => {
            match auto_install() {
                Ok(_) => {
                    display_help();
                    Ok(())
                },
                Err(e) => {
                    eprintln!("Error: {}", e);
                    display_help();
                    Ok(())
                }
            }
        }
    }
}