use anyhow::{Context, Result};
use chrono::Local;
use crossbeam_channel::{bounded, Receiver, Sender};
use dirs;
use minimo::showln;
use parking_lot::Mutex;
use regex::Regex;
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::ffi::c_int;
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::windows::ffi::OsStrExt;
use std::process::Command;
use std::time::{Duration, Instant};
use std::{env, fs, io, mem, ptr, thread};
use winapi::um::minwinbase::OVERLAPPED;
use winapi::um::synchapi::WaitForSingleObject;
use winreg::enums::*;
use winreg::RegKey;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use winapi::shared::minwindef::{DWORD, FALSE, LPARAM, LPDWORD, LPVOID, LRESULT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::fileapi::*;
use winapi::um::handleapi::*;
use winapi::um::processthreadsapi::{GetCurrentThreadId, OpenProcess};
use winapi::um::winbase::*;
use winapi::um::winnt::{
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, HANDLE,
};
use winapi::um::winuser::*;
use once_cell::sync::Lazy;

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct Config {
    matches: Vec<Match>,
    #[serde(default)]
    backend: BackendConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type")]
enum Match {
    Simple {
        trigger: String,
        replace: String,
        #[serde(default)]
        propagate_case: bool,
        #[serde(default)]
        word: bool,
    },
    Regex {
        pattern: String,
        replace: String,
    },
    Dynamic {
        trigger: String,
        action: String,
    },
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct BackendConfig {
    #[serde(default = "default_key_delay")]
    key_delay: u64,
}

fn default_key_delay() -> u64 {
    10
}

struct AppState {
    config: Arc<Mutex<Config>>,
    current_text: Arc<Mutex<Rope>>,
    last_key_time: Arc<Mutex<Instant>>,
    shift_pressed: Arc<AtomicBool>,
    caps_lock_on: Arc<AtomicBool>,
    killswitch: Arc<AtomicBool>,
}

impl AppState {
    fn new() -> Result<Self> {
        let config = load_config().unwrap_or_else(|e| {
            eprintln!("Error loading config: {}", e);
            Config::default()
        });

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            current_text: Arc::new(Mutex::new(Rope::new())),
            last_key_time: Arc::new(Mutex::new(Instant::now())),
            shift_pressed: Arc::new(AtomicBool::new(false)),
            caps_lock_on: Arc::new(AtomicBool::new(false)),
            killswitch: Arc::new(AtomicBool::new(false)),
        })
    }
}

enum Message {
    KeyEvent(DWORD, WPARAM, LPARAM),
    ConfigReload,
    Quit,
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "install" => return install(),
            "uninstall" => return uninstall(),
            _ => {
                eprintln!("Unknown command. Use 'install' or 'uninstall'.");
                return Ok(());
            }
        }
    }

    let app_state = Arc::new(AppState::new()?);
    let (sender, receiver) = bounded(100);

    let config_watcher = thread::spawn({
        let sender = sender.clone();
        move || watch_config(sender)
    });

    let keyboard_listener = thread::spawn({
        let sender = sender.clone();
        move || listen_keyboard(sender)
    });

    main_loop(app_state, &receiver)?;

    sender.send(Message::Quit).unwrap();
    config_watcher.join().unwrap()?;
    keyboard_listener.join().unwrap()?;

    Ok(())
}

fn install() -> Result<()> {
    println!("Installing Textra...");

    let exe_path = env::current_exe()?;
    let install_dir = dirs::data_local_dir().unwrap().join("Textra");
    fs::create_dir_all(&install_dir)?;
    let install_path = install_dir.join("textra.exe");
    fs::copy(&exe_path, &install_path)?;

    add_to_path(&install_dir)?;
    set_autostart(&install_path)?;
    create_uninstaller(&install_dir)?;
    start_background(&install_path)?;

    println!("Textra has been installed and started. It will run automatically at startup.");
    println!("To uninstall, run 'textra uninstall'.");
    Ok(())
}

fn uninstall() -> Result<()> {
    println!("Uninstalling Textra...");

    stop_running_instance()?;
    remove_from_path()?;
    remove_autostart()?;

    let install_dir = dirs::data_local_dir().unwrap().join("Textra");
    fs::remove_dir_all(&install_dir)?;

    println!("Textra has been uninstalled successfully.");
    Ok(())
}

fn create_uninstaller(install_dir: &Path) -> Result<()> {
    let uninstaller_path = install_dir.join("uninstall.bat");
    let uninstaller_content = format!(
        "@echo off\n\
        taskkill /F /IM textra.exe\n\
        \"{0}\" uninstall\n\
        rmdir /S /Q \"{1}\"\n",
        install_dir.join("textra.exe").display(),
        install_dir.display()
    );
    fs::write(uninstaller_path, uninstaller_content)?;
    Ok(())
}

fn stop_running_instance() -> Result<()> {
    Command::new("taskkill")
        .args(&["/F", "/IM", "textra.exe"])
        .output()?;
    Ok(())
}

fn remove_from_path() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu.create_subkey("Environment")?;
    let path: String = env.get_value("PATH")?;
    let install_dir = dirs::data_local_dir().unwrap().join("Textra");
    let new_path: Vec<&str> = path
        .split(';')
        .filter(|&p| p != install_dir.to_str().unwrap())
        .collect();
    let new_path = new_path.join(";");
    env.set_value("PATH", &new_path)?;

    unsafe {
        SendMessageTimeoutA(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            "Environment\0".as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            ptr::null_mut(),
        );
    }
    showln!(
        gray_dim,
        "removed ",
        yellow_bold,
        "PATH",
        gray_dim,
        " entry"
    );

    Ok(())
}

fn remove_autostart() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let (key, _) = hkcu.create_subkey(path)?;
    key.delete_value("Textra")?;
    showln!(
        gray_dim,
        "removed ",
        yellow_bold,
        "autostart",
        gray_dim,
        " entry"
    );
    Ok(())
}

fn add_to_path(install_dir: &Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu.create_subkey("Environment")?;
    let path: String = env.get_value("PATH")?;
    let new_path = format!("{};{}", path, install_dir.to_string_lossy());
    env.set_value("PATH", &new_path)?;

    unsafe {
        SendMessageTimeoutA(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            "Environment\0".as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            ptr::null_mut(),
        );
    }
    showln!(
        gray_dim,
        "added ",
        yellow_bold,
        "textra",
        gray_dim,
        "to the ",
        green_bold,
        "PATH",
        gray_dim,
        " environment variable."
    );
    Ok(())
}

fn set_autostart(install_path: &Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let (key, _) = hkcu.create_subkey(path)?;
    key.set_value("Textra", &install_path.to_string_lossy().to_string())?;
    showln!(
        gray_dim,
        "registered ",
        yellow_bold,
        "textra",
        gray_dim,
        "for ",
        green_bold,
        "autostart",
        gray_dim,
        " in the registry."
    );
    Ok(())
}

fn start_background(install_path: &Path) -> Result<()> {
    Command::new(install_path)
        .creation_flags(winapi::um::winbase::CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}

fn main_loop(app_state: Arc<AppState>, receiver: &Receiver<Message>) -> Result<()> {
    while let Ok(msg) = receiver.recv() {
        match msg {
            Message::KeyEvent(vk_code, w_param, l_param) => {
                if let Err(e) = handle_key_event(Arc::clone(&app_state), vk_code, w_param, l_param)
                {
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

fn load_config() -> Result<Config> {
    let config_path = get_config_path()?;
    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
    let config: Config = serde_yaml::from_str(&config_str)
        .with_context(|| format!("Failed to parse config file: {:?}", config_path))?;

    minimo::showln!(
        yellow_bold,
        "┌─",
        white_bold,
        " TEXTRA",
        yellow_bold,
        " ───────────────────────────────────────────────────────"
    );
    minimo::showln!(yellow_bold, "│ ", green_bold, config_path.display());
    if !config.matches.is_empty() {
        for match_rule in &config.matches {
            let (trigger, replace) = match match_rule {
                Match::Simple {
                    trigger, replace, ..
                } => (trigger, replace),
                Match::Regex { pattern, replace } => (pattern, replace),
                Match::Dynamic { trigger, action } => (trigger, action),
            };
            let trimmed = minimo::text::chop(replace, 50 - trigger.len())[0].clone();

            minimo::showln!(
                yellow_bold,
                "│ ",
                yellow_bold,
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
    minimo::showln!(
        yellow_bold,
        "└───────────────────────────────────────────────────────────────"
    );
    minimo::showln!(gray_dim, "");
    Ok(config)
}

fn get_config_path() -> Result<PathBuf> {
    let current_dir = env::current_dir()?;
    let current_dir_config = current_dir.join("config.yaml");
    if current_dir_config.exists() {
        return Ok(current_dir_config);
    }

    if current_dir.file_name().unwrap() == "textra" {
        let config_file = current_dir.join("config.yaml");
        create_default_config(&config_file)?;
        return Ok(config_file);
    }

    if let Some(home_dir) = dirs::document_dir() {
        let home_config_dir = home_dir.join("textra");
        let home_config_file = home_config_dir.join("config.yaml");
        if home_config_file.exists() {
            return Ok(home_config_file);
        }
    }

    let new_config_dir = current_dir.join("textra");
    fs::create_dir_all(&new_config_dir).context("Failed to create config directory")?;
    let new_config_file = new_config_dir.join("config.yaml");
    create_default_config(&new_config_file)?;
    Ok(new_config_file)
}

fn create_default_config(path: &Path) -> Result<()> {
    let default_config = Config {
        matches: vec![
            Match::Simple {
                trigger: "btw".to_string(),
                replace: "by the way".to_string(),
                propagate_case: false,
                word: false,
            },
            Match::Dynamic {
                trigger: ":date".to_string(),
                action: "{{date}}".to_string(),
            },
        ],
        backend: BackendConfig { key_delay: 10 },};
        let yaml = serde_yaml::to_string(&default_config)?;
        fs::write(path, yaml).context("Failed to write default config file")?;
        Ok(())
    }
    
    fn watch_config(sender: Sender<Message>) -> Result<()> {
        let config_path = get_config_path()?;
        let config_dir = config_path.parent().unwrap();
    
        unsafe {
            let dir_handle = CreateFileW(
                config_dir
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
                    .as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            );
    
            if dir_handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error().into());
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
                    return Err(io::Error::last_os_error().into());
                }
    
                let event = WaitForSingleObject(dir_handle, INFINITE);
                if event != WAIT_OBJECT_0 {
                    return Err(io::Error::last_os_error().into());
                }
    
                sender.send(Message::ConfigReload).unwrap();
            }
        }
    }
    
    static GLOBAL_SENDER: Lazy<Mutex<Option<Sender<Message>>>> = Lazy::new(|| Mutex::new(None));
    
    fn set_global_sender(sender: Sender<Message>) {
        let mut global_sender = GLOBAL_SENDER.lock();
        *global_sender = Some(sender);
    }
    
    unsafe extern "system" fn keyboard_hook_proc(
        code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if code >= 0 {
            let kb_struct = *(l_param as *const KBDLLHOOKSTRUCT);
            let vk_code = kb_struct.vkCode;
    
            if let Some(sender) = GLOBAL_SENDER.lock().as_ref() {
                // Use try_send to avoid blocking
                let _ = sender.try_send(Message::KeyEvent(vk_code, w_param, l_param));
            }
        }
    
        CallNextHookEx(ptr::null_mut(), code, w_param, l_param)
    }
    
    fn listen_keyboard(sender: Sender<Message>) -> Result<()> {
        set_global_sender(sender);
    
        unsafe {
            let hook = SetWindowsHookExA(WH_KEYBOARD_LL, Some(keyboard_hook_proc), ptr::null_mut(), 0);
    
            if hook.is_null() {
                return Err(io::Error::last_os_error().into());
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
    
    fn handle_key_event(
        app_state: Arc<AppState>,
        vk_code: DWORD,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> Result<()> {
        let now = Instant::now();
    
        match w_param as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let mut last_key_time = app_state.last_key_time.lock();
                if now.duration_since(*last_key_time) > Duration::from_millis(500) {
                    app_state.current_text.lock().remove(..);
                }
                *last_key_time = now;
    
                match vk_code as i32 {
                    VK_ESCAPE => {
                        app_state.killswitch.store(true, Ordering::SeqCst);
                    }
                    VK_SHIFT => {
                        app_state.shift_pressed.store(true, Ordering::SeqCst);
                    }
                    VK_CAPITAL => {
                        let current = app_state.caps_lock_on.load(Ordering::SeqCst);
                        app_state.caps_lock_on.store(!current, Ordering::SeqCst);
                    }
                    _ => {
                        if let Some(c) = vk_code_to_char(
                            vk_code as i32,
                            app_state.shift_pressed.load(Ordering::SeqCst),
                            app_state.caps_lock_on.load(Ordering::SeqCst),
                        ) {
                            let mut current_text = app_state.current_text.lock();
                            let txtlen = current_text.len_chars();
                            current_text.insert(txtlen, &c.to_string());
                            check_and_replace(&app_state, &mut current_text)?;
                        }
                    }
                }
            }
            WM_KEYUP | WM_SYSKEYUP => match vk_code as i32 {
                VK_SHIFT => {
                    app_state.shift_pressed.store(false, Ordering::SeqCst);
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
    
    fn check_and_replace(app_state: &AppState, current_text: &mut Rope) -> Result<()> {
        let immutable_current_text = current_text.to_string();
        let config = app_state.config.lock();
        for match_rule in &config.matches {
            match match_rule {
                Match::Simple {
                    trigger,
                    replace,
                    propagate_case,
                    word,
                } => {
                    if immutable_current_text.ends_with(trigger) {
                        let start = immutable_current_text.len() - trigger.len();
                        if !*word
                            || (start == 0
                                || !immutable_current_text
                                    .chars()
                                    .nth(start - 1)
                                    .unwrap()
                                    .is_alphanumeric())
                        {
                            perform_replacement(
                                current_text,
                                config.backend.key_delay,
                                &immutable_current_text[start..],
                                replace,
                                *propagate_case,
                                false,
                                app_state,
                            )?;
                            return Ok(());
                        }
                    }
                }
                Match::Regex { pattern, replace } => {
                    let regex = Regex::new(pattern)?;
                    if let Some(captures) = regex.captures(&immutable_current_text) {
                        let mut replacement = replace.clone();
                        for (i, capture) in captures.iter().enumerate().skip(1) {
                            if let Some(capture) = capture {
                                replacement = replacement.replace(&format!("${}", i), capture.as_str());
                            }
                        }
                        perform_replacement(
                            current_text,
                            config.backend.key_delay,
                            &immutable_current_text,
                            &replacement,
                            false,
                            false,
                            app_state,
                        )?;
                        return Ok(());
                    }
                }
                Match::Dynamic { trigger, action } => {
                    if immutable_current_text.ends_with(trigger) {
                        let replacement = process_dynamic_replacement(action);
                        perform_replacement(
                            current_text,
                            config.backend.key_delay,
                            trigger,
                            &replacement,
                            false,
                            true,
                            app_state,
                        )?;
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
    
    fn perform_replacement(
        current_text: &mut Rope,
        key_delay: u64,
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
    
        // Backspace the original text
        let backspace_count = original.chars().count();
        let backspaces = vec![VK_BACK; backspace_count];
        simulate_key_presses(&backspaces, key_delay);
    
        // Type the replacement
        let vk_codes = string_to_vk_codes(&final_replacement);
        simulate_key_presses(&vk_codes, key_delay);
    
        let start = current_text.len_chars() - original.len();
        current_text.remove(start..current_text.len_chars());
        current_text.insert(start, &final_replacement);
        Ok(())
    }
    
    fn string_to_vk_codes(s: &str) -> Vec<i32> {
        s.chars().map(|c| char_to_vk_code(c)).collect()
    }
    
    fn char_to_vk_code(c: char) -> i32 {
        unsafe { VkKeyScanW(c as u16) as i32 }
    }
    
    fn process_dynamic_replacement(replacement: &str) -> String {
        match replacement.to_lowercase().as_str() {
            "{{date}}" => Local::now().format("%Y-%m-%d").to_string(),
            _ => replacement.to_string(),
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
    
    fn reload_config(app_state: Arc<AppState>) -> Result<()> {
        let mut config = app_state.config.lock();
        *config = load_config()?;
        Ok(())
    }
    
    fn vk_code_to_char(vk_code: i32, shift_pressed: bool, caps_lock_on: bool) -> Option<char> {
        let mut keyboard_state = [0u8; 256];
        if shift_pressed {
            keyboard_state[VK_SHIFT as usize] = 0x80;
        }
        if caps_lock_on {
            keyboard_state[VK_CAPITAL as usize] = 0x01;
        }
    
        let mut unicode_chars = [0u16; 2];
        let result = unsafe {
            ToUnicodeEx(
                vk_code as u32,
                MapVirtualKeyA(vk_code as u32, MAPVK_VK_TO_VSC) as u32,
                keyboard_state.as_ptr(),
                unicode_chars.as_mut_ptr(),
                unicode_chars.len() as i32,
                0,
                ptr::null_mut(),
            )
        };
    
        if result == 1 || result == 2 {
            char::from_u32(unicode_chars[0] as u32)
        } else {
            // Handle special cases for punctuation and symbols
            match vk_code {
                VK_OEM_1 => Some(if shift_pressed { ':' } else { ';' }),
                VK_OEM_PLUS => Some(if shift_pressed { '+' } else { '=' }),
                VK_OEM_COMMA => Some(if shift_pressed { '<' } else { ',' }),
                VK_OEM_MINUS => Some(if shift_pressed { '_' } else { '-' }),
                VK_OEM_PERIOD => Some(if shift_pressed { '>' } else { '.' }),
                VK_OEM_2 => Some(if shift_pressed { '?' } else { '/' }),
                VK_OEM_3 => Some(if shift_pressed { '~' } else { '`' }),
                VK_OEM_4 => Some(if shift_pressed { '{' } else { '[' }),
                VK_OEM_5 => Some(if shift_pressed { '|' } else { '\\' }),
                VK_OEM_6 => Some(if shift_pressed { '}' } else { ']' }),
                VK_OEM_7 => Some(if shift_pressed { '"' } else { '\'' }),
                _ => None,
            }
        }
    }
    
    fn simulate_key_presses(vk_codes: &[i32], key_delay: u64) {
        let batch_size = 20;
        let delay = Duration::from_millis(key_delay);
        let input_count = vk_codes.len() * 2;
        let mut inputs: Vec<MaybeUninit<INPUT>> = Vec::with_capacity(input_count);
    
        // Pre-allocate the entire vector
        unsafe { inputs.set_len(input_count) };
    
        // Fill the vector without initializing
        for (i, &vk) in vk_codes.iter().enumerate() {
            let press_index = i * 2;
            let release_index = press_index + 1;
    
            // Key press event
            unsafe {
                let input = inputs[press_index].as_mut_ptr();
                (*input).type_ = INPUT_KEYBOARD;
                let ki = (*input).u.ki_mut();
                ki.wVk = vk as u16;
                ki.dwFlags = 0;
            }
    
            // Key release event
            unsafe {
                let input = inputs[release_index].as_mut_ptr();
                (*input).type_ = INPUT_KEYBOARD;
                let ki = (*input).u.ki_mut();
                ki.wVk = vk as u16;
                ki.dwFlags = KEYEVENTF_KEYUP;
            }
        }
    
        // Send inputs in batches
        for chunk in inputs.chunks(batch_size * 2) {
            unsafe {
                SendInput(
                    chunk.len() as u32,
                    chunk.as_ptr() as *mut INPUT,
                    size_of::<INPUT>() as c_int,
                );
            }
            // Add a small delay between batches for more natural input
            thread::sleep(delay);
        }
    }