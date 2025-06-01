//! Textra Core Daemon
//! 
//! This binary is responsible for:
//! 1. Monitoring keyboard input
//! 2. Detecting triggers for text expansion
//! 3. Detecting double-shift keypress to show the overlay
//! 4. Replacing text based on configuration

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
    thread,
    mem,
    ptr,
};

use anyhow::Context; // Keep anyhow::Context for its specific use case
use tracing::{debug, error, info, warn}; // Replaced log with tracing
use tracing_subscriber::{EnvFilter, FmtSubscriber}; // Added for tracing setup

use textra::{
    get_config_path, ipc, keyboard::{self, KeyCode, KeyEvent}, load_config, replacement::{execute_code, process_dynamic_replacement, propagate_case, KEY_DELAY}, IpcMessage, KeyModifiers,  KeyboardInput, Replacement, Result, TextraConfig, TextraError, TextraRule, WindowsKeyboardApi, DAEMON_PIPE_NAME, MAX_TEXT_LENGTH, OVERLAY_PIPE_NAME // Import renamed WindowsKeyboard implementation
 
};

// Main application state
struct AppState {
    config: Arc<Mutex<TextraConfig>>,
    current_text: Arc<Mutex<VecDeque<char>>>,
    
    last_key_time: Arc<Mutex<Instant>>,
    shift_pressed: Arc<AtomicBool>,
    ctrl_pressed: Arc<AtomicBool>,
    alt_pressed: Arc<AtomicBool>,
    caps_lock_on: Arc<AtomicBool>,
    killswitch: Arc<AtomicBool>,
    
    // Track shift key for double-shift detection
    last_shift_time: Arc<Mutex<Option<Instant>>>,
    overlay_visible: Arc<AtomicBool>,
    keyboard_api: Arc<dyn KeyboardInput>, // Use the KeyboardInput trait since we need typing functionality
}

impl AppState {
    // Modified to accept KeyboardApi
    fn new(keyboard_api: Arc<dyn KeyboardInput>) -> Result<Self> {
        let config = load_config().context("Failed to load configuration in AppState::new")?;

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            current_text: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_TEXT_LENGTH))),
            
            last_key_time: Arc::new(Mutex::new(Instant::now())),
            shift_pressed: Arc::new(AtomicBool::new(false)),
            ctrl_pressed: Arc::new(AtomicBool::new(false)),
            alt_pressed: Arc::new(AtomicBool::new(false)),
            caps_lock_on: Arc::new(AtomicBool::new(false)),
            killswitch: Arc::new(AtomicBool::new(false)),
            
            last_shift_time: Arc::new(Mutex::new(None)),
            overlay_visible: Arc::new(AtomicBool::new(false)),
            keyboard_api, // Store the provided KeyboardInput implementation
        })
    }
    
    fn update_config(&self) -> Result<()> {
        info!("Attempting to reload configuration...");
        let new_config = load_config().context("Failed to reload configuration in update_config")?;
        
        {
            let mut config_lock = self.config.lock().map_err(|_| TextraError::Process("Config lock poisoned in update_config".to_string()))?;
            *config_lock = new_config.clone();
        }
        
        info!("Configuration reloaded, notifying overlay.");
        ipc::send_message(OVERLAY_PIPE_NAME, &IpcMessage::ConfigReloaded { config: new_config })
            .map_err(|e| {
                error!("Failed to send ConfigReloaded message to overlay: {}", e);
                TextraError::Ipc(format!("Failed to notify overlay of config change: {}", e))
            })?;
        
        Ok(())
    }
    
    fn handle_key_event(&self, event: KeyEvent) -> Result<()> {
        let now = Instant::now();
        
        match event {
            KeyEvent::KeyDown(code) => {
                let mut last_key_time = self.last_key_time.lock().map_err(|_| TextraError::Process("last_key_time lock poisoned".to_string()))?;
                
                if now.duration_since(*last_key_time) > Duration::from_millis(1000) {
                    self.current_text.lock().map_err(|_| TextraError::Process("current_text lock poisoned for clear".to_string()))?.clear();
                }
                
                *last_key_time = now;
                
                match code {
                    KeyCode::Escape => {
                        self.killswitch.store(true, Ordering::SeqCst);
                    }
                    KeyCode::Shift => {
                        self.shift_pressed.store(true, Ordering::SeqCst);
                        
                        let mut last_shift_lock = self.last_shift_time.lock().map_err(|_| TextraError::Process("last_shift_time lock poisoned".to_string()))?;
                        
                        if let Some(prev_time) = *last_shift_lock {
                            if now.duration_since(prev_time) < Duration::from_millis(500) {
                                if !self.overlay_visible.load(Ordering::SeqCst) {
                                    debug!("Double shift detected, showing overlay");
                                    self.show_overlay()?;
                                }
                                *last_shift_lock = None;
                            } else {
                                *last_shift_lock = Some(now);
                            }
                        } else {
                            *last_shift_lock = Some(now);
                        }
                    }
                    KeyCode::Control => self.ctrl_pressed.store(true, Ordering::SeqCst),
                    KeyCode::Alt => self.alt_pressed.store(true, Ordering::SeqCst),
                    KeyCode::CapsLock => {
                        let current = self.caps_lock_on.load(Ordering::SeqCst);
                        self.caps_lock_on.store(!current, Ordering::SeqCst);
                    }
                    KeyCode::Backspace => {
                        if !self.overlay_visible.load(Ordering::SeqCst) {
                            self.current_text.lock().map_err(|_| TextraError::Process("current_text lock poisoned for backspace".to_string()))?.pop_back();
                        }
                    }
                    KeyCode::Char(c) => {
                        if self.overlay_visible.load(Ordering::SeqCst) {
                            return Ok(());
                        }
                        
                        if self.ctrl_pressed.load(Ordering::SeqCst) && (c == 'v' || c == 'V') {
                             debug!("Ctrl+V detected, clearing current_text buffer.");
                            self.current_text.lock().map_err(|_| TextraError::Process("current_text lock poisoned for paste".to_string()))?.clear();
                            return Ok(());
                        }
                        
                        let mut current_text_lock = self.current_text.lock().map_err(|_| TextraError::Process("current_text lock poisoned for char input".to_string()))?;
                        current_text_lock.push_back(c);
                        
                        if current_text_lock.len() > MAX_TEXT_LENGTH {
                            current_text_lock.pop_front();
                        }
                        
                        let text: String = current_text_lock.iter().collect();
                        drop(current_text_lock); 

                        if let Some((trigger, replacement_val)) = self.find_replacement(&text)? {
                            self.perform_replacement(
                                &trigger,
                                &replacement_val,
                                self.shift_pressed.load(Ordering::SeqCst),
                                self.caps_lock_on.load(Ordering::SeqCst),
                            )?;
                            
                            let mut current_text_lock_after = self.current_text.lock().map_err(|_| TextraError::Process("current_text lock poisoned post-replacement".to_string()))?;
                            for _ in 0..trigger.len() {
                                current_text_lock_after.pop_back();
                            }
                            // After replacement, the typed text is simulated, not directly added to current_text.
                            // current_text should reflect the trigger being removed.
                            // The replacement text itself isn't added back to current_text buffer
                            // as it's assumed to be typed into the active application.
                            // So, we don't add replacement_val.chars() back here.
                        }
                    }
                    _ => {}
                }
            }
            KeyEvent::KeyUp(code) => {
                match code {
                    KeyCode::Shift => self.shift_pressed.store(false, Ordering::SeqCst),
                    KeyCode::Control => self.ctrl_pressed.store(false, Ordering::SeqCst),
                    KeyCode::Alt => self.alt_pressed.store(false, Ordering::SeqCst),
                    KeyCode::Escape => {
                        if self.overlay_visible.load(Ordering::SeqCst) {
                            self.hide_overlay()?;
                        }
                        self.killswitch.store(false, Ordering::SeqCst);
                    }
                    _ => {}
                }
            }
        }
        
        Ok(())
    }
    
    fn find_replacement(&self, text: &str) -> Result<Option<(String, String)>> {
        let config_lock = self.config.lock().map_err(|_| TextraError::Process("Config lock poisoned in find_replacement".to_string()))?;
        
        for rule in &config_lock.rules {
            for trigger in &rule.triggers {
                if text.ends_with(trigger) {
                    let replacement_text = match &rule.replacement {
                        Replacement::Simple(text_val) => text_val.clone(),
                        Replacement::Multiline(text_val) => text_val.clone(),
                        Replacement::Code { language, content } => {
                            match execute_code(language, content) {
                                Ok(output) => output,
                                Err(e) => {
                                    error!("Error executing code replacement for trigger '{}': {}", trigger, e);
                                    format!("Error executing code: {}", e) 
                                }
                            }
                        }
                    };
                    debug!("Found replacement for trigger '{}': '{}'", trigger, replacement_text);
                    return Ok(Some((trigger.clone(), replacement_text)));
                }
            }
        }
        
        Ok(None)
    }
    
    fn perform_replacement(
        &self,
        trigger: &str,
        replacement: &str,
        shift_pressed: bool,
        caps_lock_on: bool,
    ) -> Result<()> {
        let final_replacement = if replacement.contains("{{") {
            process_dynamic_replacement(replacement)
        } else {
            propagate_case(trigger, replacement)
        };
        
        if self.killswitch.load(Ordering::SeqCst) {
            info!("Killswitch active, skipping replacement for trigger '{}'", trigger);
            return Ok(());
        }
        
        debug!("Performing replacement: trigger='{}', replacement='{}', final='{}'", trigger, replacement, final_replacement);
        
        // Use KeyboardApi trait
        self.keyboard_api.delete_chars(trigger.chars().count(), KEY_DELAY)
            .context(format!("Failed to delete trigger text for '{}'", trigger))?;
        
        let modifiers = KeyModifiers {
            shift_pressed,
            caps_lock_on,
            ctrl_pressed: false,
            alt_pressed: false,
        };
        
        self.keyboard_api.type_text(&final_replacement, modifiers, KEY_DELAY)
            .context(format!("Failed to type replacement text for '{}'", final_replacement))?;
        
        Ok(())
    }
    
    fn show_overlay(&self) -> Result<()> {
        info!("Showing overlay");
        ipc::send_message(OVERLAY_PIPE_NAME, &IpcMessage::ShowOverlay)
            .context("Failed to send ShowOverlay message")?;
            
        self.overlay_visible.store(true, Ordering::SeqCst);
        Ok(())
    }
    
    fn hide_overlay(&self) -> Result<()> {
        info!("Hiding overlay");
        ipc::send_message(OVERLAY_PIPE_NAME, &IpcMessage::HideOverlay)
            .context("Failed to send HideOverlay message")?;
            
        self.overlay_visible.store(false, Ordering::SeqCst);
        Ok(())
    }
    
    fn handle_template_selected(&self, text: String) -> Result<()> {
        info!("Template selected: {}", text);
        self.hide_overlay().context("Failed to hide overlay before typing template")?;
        
        // Use KeyboardApi trait
        let modifiers = KeyModifiers {
            shift_pressed: self.shift_pressed.load(Ordering::SeqCst),
            caps_lock_on: self.caps_lock_on.load(Ordering::SeqCst),
            ctrl_pressed: false,
            alt_pressed: false,
        };
        
        self.keyboard_api.type_text(&text, modifiers, KEY_DELAY)
            .context(format!("Failed to type selected template: {}", text))?;
        
        Ok(())
    }
}

// Keyboard hook to capture all keypresses
type KeyboardCallback = Box<dyn Fn(KeyEvent) + Send + 'static>;
static mut HOOK_STATE: Option<KeyboardCallback> = None;

unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    w_param: winapi::shared::minwindef::WPARAM,
    l_param: winapi::shared::minwindef::LPARAM,
) -> winapi::shared::minwindef::LRESULT {
    if code >= 0 && !keyboard::is_generating_input() { // is_generating_input is still from textra::keyboard
        let kb_struct_ptr = l_param as *const winapi::um::winuser::KBDLLHOOKSTRUCT;
        if kb_struct_ptr.is_null() {
            error!("KBDLLHOOKSTRUCT pointer is null in keyboard_hook_proc");
            return winapi::um::winuser::CallNextHookEx(ptr::null_mut(), code, w_param, l_param);
        }
        let kb_struct = *kb_struct_ptr;
        let vk_code = kb_struct.vkCode;
        
        let key_code_res = key_code_from_windows(vk_code);
        
        if let Ok(key_code) = key_code_res {
            let event = match w_param as u32 {
                winapi::um::winuser::WM_KEYDOWN | winapi::um::winuser::WM_SYSKEYDOWN => KeyEvent::KeyDown(key_code),
                winapi::um::winuser::WM_KEYUP | winapi::um::winuser::WM_SYSKEYUP => KeyEvent::KeyUp(key_code),
                _ => return winapi::um::winuser::CallNextHookEx(ptr::null_mut(), code, w_param, l_param),
            };
            
            if let Some(callback) = &HOOK_STATE {
                (callback)(event);
            }
        } else {
            warn!("Failed to convert vk_code {} to KeyCode", vk_code);
        }
    }
    
    winapi::um::winuser::CallNextHookEx(ptr::null_mut(), code, w_param, l_param)
}

unsafe fn key_code_from_windows(vk_code: u32) -> Result<KeyCode> {
    match vk_code as i32 {
        winapi::um::winuser::VK_SHIFT | winapi::um::winuser::VK_LSHIFT | winapi::um::winuser::VK_RSHIFT => Ok(KeyCode::Shift),
        winapi::um::winuser::VK_CONTROL | winapi::um::winuser::VK_LCONTROL | winapi::um::winuser::VK_RCONTROL => Ok(KeyCode::Control),
        winapi::um::winuser::VK_MENU | winapi::um::winuser::VK_LMENU | winapi::um::winuser::VK_RMENU => Ok(KeyCode::Alt),
        winapi::um::winuser::VK_ESCAPE => Ok(KeyCode::Escape),
        winapi::um::winuser::VK_BACK => Ok(KeyCode::Backspace),
        winapi::um::winuser::VK_CAPITAL => Ok(KeyCode::CapsLock),
        0x0D => Ok(KeyCode::Enter), 
        0x09 => Ok(KeyCode::Tab),   
        _ => {
            let mut keyboard_state: [u8; 256] = [0; 256];
            let hkl = winapi::um::winuser::GetKeyboardLayout(0);
            let scan_code = winapi::um::winuser::MapVirtualKeyExW(vk_code, winapi::um::winuser::MAPVK_VK_TO_VSC_EX, hkl) as u16;
            
            let mut char_buffer: [u16; 2] = [0; 2];
            let result = winapi::um::winuser::ToUnicodeEx(
                vk_code,
                scan_code as u32,
                keyboard_state.as_ptr(),
                char_buffer.as_mut_ptr(),
                2, 
                0, 
                hkl,
            );
            
            if result == 1 {
                if let Some(c) = char::from_u32(char_buffer[0] as u32) {
                    Ok(KeyCode::Char(c))
                } else {
                    warn!("Failed to convert char_buffer[0] to char: {}", char_buffer[0]);
                    Ok(KeyCode::Other(vk_code))
                }
            } else {
                debug!("ToUnicodeEx result: {} for vk_code: {}", result, vk_code);
                Ok(KeyCode::Other(vk_code))
            }
        }
    }
}


fn start_keyboard_hook<F>(callback: F) -> Result<()>
where
    F: Fn(KeyEvent) + Send + 'static,
{
    unsafe {
        HOOK_STATE = Some(Box::new(callback));
        
        let hook = winapi::um::winuser::SetWindowsHookExW(
            winapi::um::winuser::WH_KEYBOARD_LL,
            Some(keyboard_hook_proc),
            ptr::null_mut(), 
            0, 
        );
        
        if hook.is_null() {
            let error_code = std::io::Error::last_os_error();
            error!("Failed to set keyboard hook: {}", error_code);
            return Err(TextraError::KeyboardHook { source: error_code });
        }
        info!("Keyboard hook set successfully.");
        
        let mut msg: winapi::um::winuser::MSG = mem::zeroed();
        loop {
            match winapi::um::winuser::GetMessageA(&mut msg, ptr::null_mut(), 0, 0) {
                -1 => { 
                    let error_code = std::io::Error::last_os_error();
                    error!("Error in GetMessageA loop: {}", error_code);
                    winapi::um::winuser::UnhookWindowsHookEx(hook);
                    return Err(TextraError::KeyboardHook { source: error_code });
                }
                0 => { 
                    info!("WM_QUIT received, exiting message loop.");
                    break;
                }
                _ => { 
                    winapi::um::winuser::TranslateMessage(&msg);
                    winapi::um::winuser::DispatchMessageA(&msg);
                }
            }
        }
        
        info!("Unhooking keyboard hook.");
        winapi::um::winuser::UnhookWindowsHookEx(hook);
        Ok(())
    }
}

fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|e| {
                    eprintln!("Failed to parse RUST_LOG environment variable: {}. Using default 'info' level.", e);
                    EnvFilter::new("info") 
                })
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| TextraError::Process(format!("Failed to set global default subscriber: {}", e)))?;

    info!("Textra Core Daemon starting...");
    
    // Instantiate real keyboard implementation
    let keyboard_api = Arc::new(WindowsKeyboardApi::new()) as Arc<dyn KeyboardInput>;
    let app_state = Arc::new(AppState::new(keyboard_api).context("Failed to create AppState in main")?);
    
    let app_state_ipc = Arc::clone(&app_state);
    ipc::listen(DAEMON_PIPE_NAME, move |message : IpcMessage| {
        let app_state_clone = Arc::clone(&app_state_ipc);
        match message {
            IpcMessage::TemplateSelected { text } => {
                app_state_clone.handle_template_selected(text)?;
            }
            IpcMessage::UpdateConfig => {
                app_state_clone.update_config()?;
            }
            IpcMessage::StatusRequest => {
                debug!("Received StatusRequest, sending StatusResponse.");
                ipc::send_message(OVERLAY_PIPE_NAME, &IpcMessage::StatusResponse {
                    daemon_running: true,
                    overlay_running: app_state_clone.overlay_visible.load(Ordering::SeqCst),
                    autostart_enabled: true, // Placeholder
                })?;
            }
            _ => {
                debug!("Ignoring unhandled IPC message: {:?}", message);
            }
        }
        Ok(())
    }).context("IPC listener setup failed")?;
    
    let config_path = get_config_path().context("Failed to get config path for watcher")?;
    let app_state_watcher = Arc::clone(&app_state);
    
    info!("Setting up config file watcher for: {:?}", config_path);
    let _watcher_thread = thread::spawn(move || {
        use notify::{Watcher, RecursiveMode, RecommendedWatcher, event::AccessKind};
        
        let (tx, rx) = std::sync::mpsc::channel();
        
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                Ok(event) => {
                    if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() || matches!(event.kind, notify::EventKind::Access(AccessKind::Close(_))) {
                         debug!("Config watcher event: {:?}", event);
                        if event.paths.iter().any(|p| p.ends_with(textra::CONFIG_FILE_NAME)) {
                            if tx.send(()).is_err() {
                                error!("Config watcher: Failed to send signal, receiver likely dropped.");
                            }
                        }
                    }
                }
                Err(e) => error!("Config watcher error: {:?}", e),
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                error!("Failed to create config file watcher: {}", e);
                return;
            }
        };
        
        if let Some(parent_dir) = config_path.parent() {
            if let Err(e) = watcher.watch(parent_dir, RecursiveMode::NonRecursive) {
                 error!("Failed to watch config directory {:?}: {}", parent_dir, e);
                 return;
            }
            info!("Watching directory {:?} for configuration changes.", parent_dir);
        } else {
            error!("Could not get parent directory for config path: {:?}", config_path);
            return;
        }
            
        while rx.recv().is_ok() {
            info!("Config file change detected, attempting reload.");
            if let Err(e) = app_state_watcher.update_config() {
                error!("Failed to reload config after watcher detected change: {}", e);
            } else {
                info!("Configuration reloaded successfully via watcher.");
            }
        }
        info!("Config watcher thread finished.");
    });
    
    let app_state_kb = Arc::clone(&app_state);
    let keyboard_handler = move |event| {
        if let Err(e) = app_state_kb.handle_key_event(event) {
            error!("Error handling key event: {}", e);
        }
    };
    
    info!("Starting keyboard hook...");
    start_keyboard_hook(keyboard_handler).context("Keyboard hook failed to start")?;
    
    info!("Textra Core Daemon running. Main loop initiated.");
    info!("Textra Core Daemon main thread exiting (keyboard hook loop finished).");
    Ok(())
}
 
    
    use textra::{ OverlayConfig,  }; // Use exported types from lib.rs
    use std::collections::HashMap;

    // Helper to create a default AppState for testing, now takes a KeyboardApi
    fn create_test_app_state(keyboard_api: Arc<dyn KeyboardInput>) -> AppState {
        let config = TextraConfig {
            metadata: HashMap::new(),
            documentation: Vec::new(),
            rules: vec![
                TextraRule {
                    triggers: vec!["btw".to_string()],
                    replacement: Replacement::Simple("by the way".to_string()),
                    description: None,
                    category: None,
                },
                TextraRule {
                    triggers: vec![":date".to_string()],
                    replacement: Replacement::Simple("{{date}}".to_string()), // Will be processed
                    description: None,
                    category: None,
                },
            ],
            overlay: OverlayConfig::default(),
        };
        AppState::new(keyboard_api).expect("Failed to create test AppState")
    }
 