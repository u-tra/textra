//! Textra - Text Expansion Utility
//!
//! This library contains common functionality used across all Textra components

pub mod errors;
pub mod keyboard_api;
pub use keyboard_api::{
    KeyboardMonitor, KeyboardInput, KeyModifiers, HealthStatus,
    WindowsKeyboard as WindowsKeyboardApi, // Rename for clarity
};
#[cfg(test)]
pub use keyboard_api::mock::{MockKeyboard as MockKeyboardApi, KeyboardAction}; // Export KeyboardAction for tests
pub use errors::{ConfigError, TextraError, Result, KeyboardError};

use std::{
    collections::{HashMap, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::Duration,
    io::Write,
    thread,
    os::windows::process::CommandExt, // Add Windows Command extension
};

use anyhow::Context; // Keep anyhow::Context for error chaining
use chrono::Local;
use lazy_static::lazy_static;
use pest::Parser;
use pest_derive::Parser;
use serde::{Serialize, Deserialize};
use interprocess::local_socket::{LocalSocketListener, LocalSocketStream};
use tracing::{debug, error, info, warn}; // Replaced log with tracing

// Constants
pub const MAX_TEXT_LENGTH: usize = 100;
pub const CONFIG_FILE_NAME: &str = "config.textra";
pub const SERVICE_NAME: &str = "textra";
// pub const PIPE_NAME: &str = r"\\.\pipe\textra"; // Not used directly, specific pipes below
pub const DAEMON_PIPE_NAME: &str = r"\\.\pipe\textra-daemon";
pub const OVERLAY_PIPE_NAME: &str = r"\\.\pipe\textra-overlay";

// ----- IPC Message Types -----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcMessage {
    // Core messages
    ShiftShiftDetected,
    TemplateSelected { text: String },
    HideOverlay,
    ShowOverlay,
    ShutdownOverlay,
    
    // Configuration messages
    ConfigReloaded { config: TextraConfig },
    UpdateConfig,
    
    // CLI control messages
    StartDaemon,
    StopDaemon,
    StartOverlay,
    StopOverlay,
    
    // Status messages
    StatusRequest,
    StatusResponse { 
        daemon_running: bool, 
        overlay_running: bool,
        autostart_enabled: bool,
    },
}

// ----- Configuration Types -----

#[derive(Debug, Clone, Serialize, Deserialize, Default)] // Added Default derive
pub struct TextraConfig {
    pub metadata: HashMap<String, String>,
    pub documentation: Vec<String>,
    pub rules: Vec<TextraRule>,
    pub overlay: OverlayConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextraRule {
    pub triggers: Vec<String>,
    pub replacement: Replacement,
    pub description: Option<String>,  // For the overlay display
    pub category: Option<String>,     // For categorizing in the overlay
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Replacement {
    Simple(String),
    Multiline(String),
    Code { language: String, content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayConfig {
    pub width: u32,
    pub height: u32,
    pub font_size: u32,
    pub font_family: String,
    pub opacity: f32,
    pub primary_color: String,
    pub secondary_color: String,
    pub text_color: String,
    pub border_radius: u32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            font_size: 14,
            font_family: "Segoe UI".to_string(),
            opacity: 0.9,
            primary_color: "#2c3e50".to_string(),
            secondary_color: "#3498db".to_string(),
            text_color: "#ecf0f1".to_string(),
            border_radius: 8,
        }
    }
}

// ----- Version -----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub parts: Vec<u32>,
}

impl Version {
    pub fn parse(version_str: &str) -> Result<Self> {
        // Remove 'v' prefix if present
        let version_str = version_str.trim_start_matches('v');
        
        // Split and parse all parts as numbers
        let parts_res: std::result::Result<Vec<u32>, _> = version_str
            .split('.')
            .map(|s| s.parse::<u32>())
            .collect();

        let parts = parts_res.map_err(|e| TextraError::VersionParse { 
            tag: version_str.to_string(), 
            reason: e.to_string() 
        })?;
        Ok(Version { parts })
    }

    pub fn to_string(&self) -> String {
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
                other_cmp => return other_cmp,
            }
        }
        std::cmp::Ordering::Equal
    }
}

// ----- Configuration Parser -----

#[derive(Parser)]
#[grammar = "textra.pest"] // Assuming textra.pest defines Rule::file, Rule::metadata, etc.
pub struct TextraParser; // Made public to be accessible by errors.rs for ConfigError::Parse

// pub type ParseError = pest::error::Error<Rule>; // Now defined in errors.rs if needed, or use TextraError::Config(ConfigError::Parse)

pub fn parse_textra_config(input: &str) -> Result<TextraConfig> {
    let mut config = TextraConfig {
        metadata: HashMap::new(),
        documentation: Vec::new(),
        rules: Vec::new(),
        overlay: OverlayConfig::default(),
    };

    let pairs = TextraParser::parse(Rule::file, input)
        .map_err(ConfigError::Parse)?; // Map pest::Error to ConfigError::Parse

    for pair in pairs {
        match pair.as_rule() {
            Rule::file => {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::metadata => parse_metadata(&mut config, inner_pair),
                        Rule::documentation => parse_documentation(&mut config, inner_pair),
                        Rule::rule => parse_rule(&mut config, inner_pair),
                        Rule::EOI => {}
                        _ => {
                            // Optionally log or handle unexpected rules
                            debug!("Unexpected rule: {:?}", inner_pair.as_rule());
                        }
                    }
                }
            }
            _ => {
                // Optionally log or handle unexpected top-level rules
                debug!("Unexpected top-level rule: {:?}", pair.as_rule());
            }
        }
    }

    Ok(config)
}

fn parse_metadata(config: &mut TextraConfig, pair: pest::iterators::Pair<Rule>) {
    let mut inner = pair.into_inner();
    // Using unwrap() here is risky if the grammar doesn't guarantee these elements.
    // Consider returning a Result or using if let for robustness.
    let key = inner.next().expect("Metadata key missing in grammar").as_str().to_string();
    let value = inner.next().expect("Metadata value missing in grammar").as_str().to_string();
    config.metadata.insert(key, value);
}

fn parse_documentation(config: &mut TextraConfig, pair: pest::iterators::Pair<Rule>) {
    let doc = pair.into_inner().next().expect("Documentation content missing").as_str().trim().to_string();
    config.documentation.push(doc);
}

fn parse_rule(config: &mut TextraConfig, pair: pest::iterators::Pair<Rule>) {
    let mut inner = pair.into_inner();
    let triggers = parse_triggers(inner.next().expect("Triggers missing in rule"));
    let replacement = parse_replacement(inner.next().expect("Replacement missing in rule"));

    // Look for a description in comments or documentation for this rule
    let description = None; // In a real implementation, we'd extract this from comments

    config.rules.push(TextraRule {
        triggers,
        replacement,
        description,
        category: None, // This could be derived from the trigger pattern or explicitly tagged
    });
}

fn parse_triggers(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .map(|trigger| trigger.as_str().trim().to_string())
        .collect()
}

fn parse_replacement(pair: pest::iterators::Pair<Rule>) -> Replacement {
    match pair.as_rule() {
        Rule::replacement => {
            let inner = pair.into_inner().next().expect("Replacement content missing");
            match inner.as_rule() {
                Rule::simple_replacement => Replacement::Simple(inner.as_str().to_string()),
                Rule::multiline_replacement => {
                    let content = inner.into_inner().next().expect("Multiline content missing").as_str().to_string();
                    Replacement::Multiline(content)
                }
                Rule::code_replacement => {
                    let mut code_inner = inner.into_inner();
                    let language = code_inner.next().expect("Code language missing").as_str().trim().to_string();
                    let content = code_inner.next().expect("Code content missing").as_str().to_string();
                    Replacement::Code { language, content }
                }
                r => unreachable!("Unexpected rule in replacement: {:?}", r),
            }
        }
        r => unreachable!("Unexpected rule for replacement: {:?}", r),
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
        let replacement_str = match &rule.replacement {
            Replacement::Simple(s) => s.to_string(),
            Replacement::Multiline(s) => format!("`{s}`"),
            Replacement::Code { language, content } => format!("```{language}\n{content}\n```"), // Added newline for```
        };
        
        // Add description as a comment if present
        if let Some(desc) = &rule.description {
            output.push_str(&format!("// {desc}\n"));
        }
        
        // Add category as a comment if present
        if let Some(cat) = &rule.category {
            output.push_str(&format!("// Category: {cat}\n"));
        }
        
        output.push_str(&format!("{triggers} => {replacement_str}\n\n"));
    }

    output
}

// ----- Configuration Management -----

pub fn get_config_path() -> Result<PathBuf> {
    let home_dir = dirs::document_dir()
        .ok_or(ConfigError::HomeDirectoryNotFound)?;
    let home_config_dir = home_dir.join("textra");
    let home_config_file = home_config_dir.join(CONFIG_FILE_NAME);

    if home_config_file.exists() {
        return Ok(home_config_file);
    }

    fs::create_dir_all(&home_config_dir)
        .map_err(|e| ConfigError::CreateConfigDir { source: e })?;
    create_default_config(&home_config_file)?; // This now returns Result
    Ok(home_config_file)
}

pub fn create_default_config(path: &Path) -> Result<()> {
    fs::write(path, DEFAULT_CONFIG)
        .map_err(|e| ConfigError::WriteDefaultConfig { source: e })?;
    Ok(())
}

pub fn load_config() -> Result<TextraConfig> {
    let config_path = get_config_path().context("Failed to get config path for loading")?;
    let config_str = fs::read_to_string(&config_path)
        .map_err(|e| ConfigError::ReadConfig { path: config_path.clone(), source: e })
        .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
    
    let mut config = parse_textra_config(&config_str)
        .context("Failed to parse configuration content")?;
    
    // Ensure we have default overlay settings
    if config.overlay.width == 0 { // This check might be too simplistic if 0 is a valid value.
        info!("Overlay width is 0, applying default overlay config.");
        config.overlay = OverlayConfig::default();
    }
    
    Ok(config)
}

// ----- IPC Utilities -----

pub mod ipc {
    use super::*; // Imports Result, TextraError, IpcMessage etc. from parent module
    use std::io::{BufRead, BufReader};
    use anyhow::Context as AnyhowContext; // Alias to avoid conflict with our Context if any

    pub fn send_message(pipe_name: &str, message: &IpcMessage) -> Result<()> {
        let connection = LocalSocketStream::connect(pipe_name)
            .map_err(|e| TextraError::Ipc(format!("Failed to connect to pipe {}: {}", pipe_name, e)))?;
            
        let message_json = serde_json::to_string(message)
            .map_err(|e| TextraError::SerdeJson { source: e })?;
        let mut writer = io::BufWriter::new(connection);
        
        writeln!(writer, "{}", message_json)
            .map_err(|e| TextraError::Ipc(format!("Failed to write message to pipe {}: {}", pipe_name, e)))?;
            
        Ok(())
    }

    /// specifically for IpcMessage 
    pub fn listen(pipe_name: &str, mut callback: impl FnMut(IpcMessage) -> Result<()> + Send + 'static) -> Result<()> {
        listen_for_messages(pipe_name, move |message_string| {
            let message: IpcMessage = serde_json::from_str(&message_string)
                .map_err(|e| TextraError::SerdeJson { source: e })?;
            
            callback(message)
                .with_context(|| format!("Error in IPC message callback: {}", message_string))?;

            Ok(())
        })
    }

    pub fn listen_for_messages<F>(pipe_name: &str, mut callback: F) -> Result<()>
    where
        F: FnMut(String) -> Result<()> + Send + 'static,
    {
        let listener = LocalSocketListener::bind(pipe_name)
            .map_err(|e| TextraError::Ipc(format!("Failed to bind to socket {}: {}", pipe_name, e)))?;
            
        info!("IPC listener started on pipe: {}", pipe_name);
        let pipe_name = pipe_name.to_string(); // Clone to owned String for thread
        
        thread::spawn(move || {
            for connection_result in listener.incoming() {
                match connection_result {
                    Ok(connection) => {
                        let peer_addr = match connection.peer_pid() {
                            Ok(addr) => format!("{:?}", addr),
                            Err(_) => "unknown".to_string(),
                        };
                        debug!("IPC connection accepted from {}", peer_addr);
                        let reader = BufReader::new(connection);
                        
                        for line_result in reader.lines() {
                            match line_result {
                                Ok(line) => {
                                    match serde_json::from_str::<IpcMessage>(&line) {
                                        Ok(message) => {
                                            debug!("Received IPC message: {:?}", message);
                                            
                                            // Convert IpcMessage to String before passing to callback
                                            let message_string = serde_json::to_string(&message).map_err(|e| {
                                                TextraError::SerdeJson { source: e }
                                            }).unwrap();
                                            
                                            if let Err(e) = callback(message_string) {
                                                error!("Error handling IPC message: {:?}", e);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Error parsing IPC message from line '{}': {:?}", line, e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Error reading line from IPC connection {}: {:?}", peer_addr, e);
                                    break; // Stop processing this connection on read error
                                }
                            }
                        }
                        debug!("IPC connection from {} ended", peer_addr);
                    }
                    Err(e) => {
                        error!("Error accepting IPC connection on pipe {}: {:?}", pipe_name, e);
                        // Consider if this should break the loop or just log and continue
                    }
                }
            }
            warn!("IPC listener loop on pipe {} has exited.", pipe_name);
        });
        
        Ok(())
    }
}

// ----- Process Management -----

pub mod process {
    use super::*; // Imports Result, TextraError etc.
    use std::process::Command;
    use std::env;
    use std::path::PathBuf;
    use std::os::windows::process::CommandExt; // Added for creation_flags
    use winapi::um::winbase::DETACHED_PROCESS;
    use anyhow::Context as AnyhowContext; // Alias for anyhow::Context

    pub fn is_process_running(name: &str) -> bool {
        let output = Command::new("tasklist")
            .arg("/FI")
            .arg(format!("IMAGENAME eq {}", name))
            .arg("/NH") // No header
            .creation_flags(DETACHED_PROCESS) // Hide console window for tasklist
            .output();
            
        match output {
            Ok(output_val) => {
                let output_str = String::from_utf8_lossy(&output_val.stdout);
                // Check if the output string contains the process name.
                // This is a basic check; more robust checks might involve parsing PIDs.
                output_str.to_lowercase().contains(&name.to_lowercase())
            }
            Err(e) => {
                warn!("Failed to execute tasklist to check if process '{}' is running: {:?}", name, e);
                false // Assume not running on error
            }
        }
    }
    
    pub fn start_process_detached(name: &str, args: &[&str]) -> Result<()> {
        let exe_path = env::current_exe()
            .map_err(|e| TextraError::CurrentExePath { source: e })?;
            
        let exe_dir = exe_path.parent()
            .ok_or(TextraError::ExeDirectory)?;
            
        let process_path = exe_dir.join(name);
        
        info!("Starting process: {:?} with args: {:?}", process_path, args);
        
        let mut command = Command::new(&process_path);
        command.args(args);
        
        // Use DETACHED_PROCESS flag for Windows to run without console
        command.creation_flags(DETACHED_PROCESS);
        
        command.spawn()
            .map_err(|e| TextraError::StartProcess { name: name.to_string(), source: e })?;
            
        Ok(())
    }
    
    pub fn stop_process(name: &str) -> Result<()> {
        info!("Attempting to stop process: {}", name);
        let output = Command::new("taskkill")
            .arg("/F") // Forcefully terminate
            .arg("/IM") // Image name
            .arg(name)
            .creation_flags(DETACHED_PROCESS) // Hide console window for taskkill
            .output()
            .map_err(|e| TextraError::StopProcess { name: name.to_string(), source: e })?;

        if output.status.success() {
            info!("Successfully sent stop command for process: {}", name);
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Failed to stop process '{}'. Status: {}. Stderr: {}", name, output.status, stderr);
            // Even if taskkill fails (e.g. process not found), we might not want to return an error here
            // depending on desired behavior. For now, let's consider it an error if taskkill itself fails.
            if !stderr.to_lowercase().contains("not found") { // Be more lenient if process simply wasn't running
                 return Err(TextraError::StopProcess { name: name.to_string(), source: io::Error::new(io::ErrorKind::Other, stderr.into_owned()) });
            }
        }
            
        Ok(())
    }
    
    pub fn get_install_dir() -> Result<PathBuf> {
        let home_dir = dirs::home_dir()
            .ok_or(ConfigError::HomeDirectoryNotFound)?; // Re-use ConfigError for this
        let install_dir = home_dir.join(".textra");
        
        std::fs::create_dir_all(&install_dir)
            .map_err(|e| ConfigError::CreateConfigDir { source: e })?; // Re-use
            
        Ok(install_dir)
    }
}

// ----- Keyboard Helper Types and Functions -----

pub mod keyboard {
    use super::*; // Imports Result, TextraError etc.
    use std::mem;
    use std::ptr;
    use winapi::{
        shared::minwindef::{WPARAM, LPARAM, LRESULT}, // Not used here but kept for consistency
        um::winuser::{
            KEYEVENTF_KEYUP, INPUT_KEYBOARD, SendInput,
            VK_SHIFT, VK_CONTROL, VK_MENU, VK_BACK,
            // Required for VkKeyScanW and ToUnicodeEx if used more broadly
            // GetKeyboardLayout, MapVirtualKeyExW, ToUnicodeEx 
        },
    };
    use anyhow::Context as AnyhowContext;

    static GENERATING_INPUT: AtomicBool = AtomicBool::new(false);

    // Key event types
    #[derive(Debug, Clone, Copy)]
    pub enum KeyEvent {
        KeyDown(KeyCode),
        KeyUp(KeyCode),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum KeyCode {
        Char(char),
        Shift,
        Control,
        Alt,
        Escape,
        Backspace,
        CapsLock,
        Enter,
        Tab,
        Other(u32), // Represents a virtual key code
    }

    // This struct seems unused in the provided code, but kept for now.
    pub struct KeyPress {
        pub modifiers: Vec<u32>, // VK codes for modifiers
        pub key: u32,            // VK code for the key
    }

    lazy_static! {
        pub static ref SYMBOL_PAIRS: HashMap<char, char> = {
            let mut m = HashMap::new();
            m.insert(';', ':'); m.insert(',', '<'); m.insert('.', '>'); m.insert('/', '?');
            m.insert('\'', '"'); m.insert('[', '{'); m.insert(']', '}'); m.insert('\\', '|');
            m.insert('`', '~'); m.insert('1', '!'); m.insert('2', '@'); m.insert('3', '#');
            m.insert('4', '$'); m.insert('5', '%'); m.insert('6', '^'); m.insert('7', '&');
            m.insert('8', '*'); m.insert('9', '('); m.insert('0', ')'); m.insert('-', '_');
            m.insert('=', '+');
            m
        };
    }

    pub fn set_generating_input(generating: bool) {
        GENERATING_INPUT.store(generating, Ordering::SeqCst);
    }

    pub fn is_generating_input() -> bool {
        GENERATING_INPUT.load(Ordering::SeqCst)
    }

    pub fn get_virtual_key_code(c: char) -> Option<u16> { // Return u16 for VK codes
        unsafe {
            let vk_scan_result = winapi::um::winuser::VkKeyScanW(c as u16);
            if vk_scan_result == -1 {
                None
            } else {
                Some((vk_scan_result & 0xFF) as u16) // VK code is the low byte
            }
        }
    }

    pub fn get_modifiers_for_char(c: char) -> Vec<u16> { // Modifiers are also VK codes (u16)
        unsafe {
            let vk_scan_result = winapi::um::winuser::VkKeyScanW(c as u16);
            if vk_scan_result == -1 {
                return vec![];
            }
            
            let shift_state = (vk_scan_result >> 8) & 0xFF; // High byte for shift state
            let mut modifiers = Vec::new();
            
            if shift_state & 1 != 0 { modifiers.push(VK_SHIFT as u16); }
            if shift_state & 2 != 0 { modifiers.push(VK_CONTROL as u16); }
            if shift_state & 4 != 0 { modifiers.push(VK_MENU as u16); }
            // Note: Other states like AltGr (Ctrl+Alt) are not handled here.
            
            modifiers
        }
    }

    pub fn send_key(key_vk: u16, modifiers_vk: &[u16], delay_ms: u64) -> Result<()> {
        let delay = Duration::from_millis(delay_ms);
        set_generating_input(true);

        let mut inputs = Vec::new();

        // Press modifiers
        for &modifier_vk_code in modifiers_vk {
            let mut input = winapi::um::winuser::INPUT { type_: INPUT_KEYBOARD, u: unsafe { std::mem::zeroed() } };
            unsafe {
                let ki = input.u.ki_mut();
                ki.wVk = modifier_vk_code;
                ki.dwFlags = 0; // Key down
            }
            inputs.push(input);
        }

        // Press main key
        let mut key_down_input = winapi::um::winuser::INPUT { type_: INPUT_KEYBOARD, u: unsafe { std::mem::zeroed() } };
        unsafe {
            let ki = key_down_input.u.ki_mut();
            ki.wVk = key_vk;
            ki.dwFlags = 0; // Key down
        }
        inputs.push(key_down_input);

        // Release main key
        let mut key_up_input = winapi::um::winuser::INPUT { type_: INPUT_KEYBOARD, u: unsafe { std::mem::zeroed() } };
        unsafe {
            let ki = key_up_input.u.ki_mut();
            ki.wVk = key_vk;
            ki.dwFlags = KEYEVENTF_KEYUP;
        }
        inputs.push(key_up_input);
        
        // Release modifiers (in reverse order of press)
        for &modifier_vk_code in modifiers_vk.iter().rev() {
            let mut input = winapi::um::winuser::INPUT { type_: INPUT_KEYBOARD, u: unsafe { std::mem::zeroed() } };
            unsafe {
                let ki = input.u.ki_mut();
                ki.wVk = modifier_vk_code;
                ki.dwFlags = KEYEVENTF_KEYUP;
            }
            inputs.push(input);
        }
        
        // Send all inputs in one go or with delays
        for input_event in inputs {
            unsafe {
                if SendInput(1, &input_event as *const _ as *mut _, mem::size_of::<winapi::um::winuser::INPUT>() as i32) == 0 {
                    set_generating_input(false); // Reset flag on error
                    return Err(TextraError::KeyboardHook{ source: io::Error::last_os_error() })
                        .with_context(|| format!("Failed to send key input for VK: {}", input_event.u.ki().wVk))?
                }
            }
            if delay_ms > 0 {
                thread::sleep(delay);
            }
        }
        
        set_generating_input(false);
        Ok(())
    }

    pub fn type_text(text: &str, _shift_pressed: bool, _caps_lock_on: bool, delay_ms: u64) -> Result<()> {
        // _shift_pressed and _caps_lock_on are not directly used here because
        // VkKeyScanW and SendInput handle the necessary shift states for characters.
        // However, they might be relevant if we were to simulate complex modifier states
        // or interact with the global shift/caps lock state more directly.
        for c in text.chars() {
            if let Some(vk_code) = get_virtual_key_code(c) {
                let modifiers = get_modifiers_for_char(c);
                send_key(vk_code, &modifiers, delay_ms)
                    .with_context(|| format!("Failed to type character '{}'", c))?;
            } else {
                warn!("No virtual key code found for character: '{}'. Skipping.", c);
            }
        }
        Ok(())
    }

    pub fn delete_chars(count: usize, delay_ms: u64) -> Result<()> {
        for i in 0..count {
            send_key(VK_BACK as u16, &[], delay_ms)
                .with_context(|| format!("Failed to send backspace (attempt {} of {})", i + 1, count))?;
        }
        Ok(())
    }
}


// ----- Text Replacement -----

pub mod replacement {
    use super::*; // Imports Result, TextraError etc.
    use anyhow::Context as AnyhowContext;

    // Key press delay in milliseconds
    pub const KEY_DELAY: u64 = 2; // Consider making this configurable
    
    // Process special replacement patterns like {{date}}, {{time}}
    pub fn process_dynamic_replacement(replacement: &str) -> String {
        let mut result = replacement.to_string();
        
        // Replace {{date}} with current date
        if result.contains("{{date}}") {
            let date = Local::now().format("%Y-%m-%d").to_string();
            result = result.replace("{{date}}", &date);
            debug!("Processed dynamic replacement: {{date}} -> {}", date);
        }
        
        // Replace {{time}} with current time
        if result.contains("{{time}}") {
            let time = Local::now().format("%H:%M:%S").to_string();
            result = result.replace("{{time}}", &time);
            debug!("Processed dynamic replacement: {{time}} -> {}", time);
        }
        
        // Add more dynamic replacements here if needed
        // e.g., {{clipboard}}, {{uuid}}, etc.
        
        result
    }
    
    // Propagate case from trigger to replacement
    pub fn propagate_case(original: &str, replacement: &str) -> String {
        if original.is_empty() || replacement.is_empty() {
            return replacement.to_string();
        }

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
    
    // Execute code-based replacements
    pub fn execute_code(language: &str, code: &str) -> Result<String> {
        info!("Executing code replacement: language='{}', code='{}'", language, code);
        match language.to_lowercase().as_str() {
            "python" => {
                let output = Command::new("python")
                    .arg("-c")
                    .arg(code)
                    .output()
                    .map_err(|e| TextraError::Process(format!("Failed to start python: {}", e)))?;
                
                if output.status.success() {
                    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    error!("Python execution failed: {}", stderr);
                    Err(TextraError::Process(format!("Python script error: {}", stderr)))
                }
            }
            "javascript" | "js" | "node" => {
                let output = Command::new("node")
                    .arg("-e")
                    .arg(code)
                    .output()
                    .map_err(|e| TextraError::Process(format!("Failed to start node: {}", e)))?;

                if output.status.success() {
                    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    error!("Node.js execution failed: {}", stderr);
                    Err(TextraError::Process(format!("Node.js script error: {}", stderr)))
                }
            }
            "rust" => {
                let dir = tempfile::Builder::new()
                    .prefix("textra_rust_exec_")
                    .tempdir()
                    .map_err(|e| TextraError::TempFile{ source: e })?;
                
                let file_path = dir.path().join("main.rs");
                let mut file = std::fs::File::create(&file_path)
                    .map_err(|e| TextraError::Io{ source: e })?;
                
                // Basic main function wrapper
                writeln!(file, "fn main() {{")?;
                writeln!(file, "    let result = {{ {} }};", code)?; // Wrap user code in a block
                writeln!(file, "    print!(\"{{}}\", result);")?; // Assume result is printable
                writeln!(file, "}}")?;
                file.flush()?;

                let output_exe_path = dir.path().join("output_exe"); // Ensure it's a unique name

                let compile_output = Command::new("rustc")
                    .arg(&file_path)
                    .arg("-o")
                    .arg(&output_exe_path)
                    .output()
                    .map_err(|e| TextraError::Process(format!("Failed to start rustc: {}", e)))?;

                if !compile_output.status.success() {
                    let stderr = String::from_utf8_lossy(&compile_output.stderr);
                    error!("Rust compilation failed: {}", stderr);
                    return Err(TextraError::Process(format!("Rust compilation error: {}", stderr)));
                }

                let exec_output = Command::new(&output_exe_path)
                    .output()
                    .map_err(|e| TextraError::Process(format!("Failed to execute compiled Rust code: {}", e)))?;
                
                if exec_output.status.success() {
                    Ok(String::from_utf8_lossy(&exec_output.stdout).trim().to_string())
                } else {
                    let stderr = String::from_utf8_lossy(&exec_output.stderr);
                    error!("Compiled Rust execution failed: {}", stderr);
                    Err(TextraError::Process(format!("Compiled Rust execution error: {}", stderr)))
                }
            }
            // Add more languages here, e.g. "powershell", "bash"
            _ => {
                warn!("Unsupported language for code replacement: {}", language);
                Err(TextraError::Process(format!("Unsupported language: {}", language)))
            }
        }
    }
}

// Default configuration
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

:tst => `twinkle twinkle little star, 
how i wonder what you are,
up above the world so high,
like a diamond in the sky`

ccc => continue writing complete code without skipping anything

// Example of a code replacement (Python)
:pydate => ```python
import datetime
print(datetime.date.today().strftime('%Y-%m-%d'))
```

// Example of a code replacement (Node.js)
:jsrand => ```javascript
console.log(Math.random().toString(36).substring(7));
```
"#;

// For overlay HTML content - will be used if src/bin/overlay.rs fails to load its own.
// This version inlines CSS and JS using include_str! for robustness.
pub fn get_default_html() -> String {
    // The paths for include_str! are relative to the current file (src/lib.rs)
    const DEFAULT_HTML_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Textra Overlay</title>
    <style>
{CSS_CONTENT}
    </style>
</head>
<body>
    <div id="overlay" class="overlay-container">
        <div class="overlay-panel">
            <div class="overlay-header">
                <h1>Textra Templates</h1>
                <button class="close-button" id="close-button">&times;</button>
            </div>
            <div class="search-bar">
                <input type="text" class="search-input" placeholder="Search templates..." id="search-input">
            </div>
            <div class="templates-container" id="templates-container">
                <!-- Categories and templates will be inserted here -->
                 <div class="empty-message">Loading default templates...</div>
            </div>
            <div class="keyboard-shortcuts">
                <p>Press ESC to close | Use arrow keys to navigate | Enter to select</p>
            </div>
        </div>
    </div>
    <script>
// Inlined from overlay.js
{JS_CONTENT}
    </script>
</body>
</html>"#;

    // Embed CSS and JS content
    // Note: These paths are relative to src/lib.rs
    let css_content = include_str!("../assets/overlay/css/styles.css");
    let js_content = include_str!("../assets/overlay/js/overlay.js");

    DEFAULT_HTML_TEMPLATE
        .replace("{CSS_CONTENT}", css_content)
        .replace("{JS_CONTENT}", js_content)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse_ok() {
        assert_eq!(Version::parse("1.2.3").unwrap().parts, vec![1, 2, 3]);
        assert_eq!(Version::parse("v0.1.0").unwrap().parts, vec![0, 1, 0]);
    }

    #[test]
    fn test_version_parse_err() {
        assert!(Version::parse("1.2.c").is_err());
        assert!(Version::parse("v1..0").is_err());
    }

    #[test]
    fn test_version_to_string() {
        let version = Version { parts: vec![1, 2, 3] };
        assert_eq!(version.to_string(), "1.2.3");
    }

    #[test]
    fn test_version_ord() {
        assert!(Version::parse("1.2.3").unwrap() < Version::parse("1.2.4").unwrap());
        assert!(Version::parse("1.3.0").unwrap() > Version::parse("1.2.10").unwrap());
        assert_eq!(Version::parse("1.2.3").unwrap(), Version::parse("1.2.3").unwrap());
        assert!(Version::parse("1.2").unwrap() < Version::parse("1.2.0").unwrap()); // 1.2.0 is greater
        assert!(Version::parse("1.2.0.0").unwrap() == Version::parse("1.2").unwrap());
    }

    #[test]
    fn test_parse_simple_config() {
        let config_content = r#"
        ///key:value
        /// doc line 1
        trigger1 => replacement1
        "#;
        let config = parse_textra_config(config_content).unwrap();
        assert_eq!(config.metadata.get("key"), Some(&"value".to_string()));
        assert_eq!(config.documentation, vec!["doc line 1"]);
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].triggers, vec!["trigger1"]);
        assert_eq!(config.rules[0].replacement, Replacement::Simple("replacement1".to_string()));
    }

    #[test]
    fn test_parse_config_empty_input() {
        let config_content = "";
        let config = parse_textra_config(config_content).unwrap();
        assert!(config.metadata.is_empty());
        assert!(config.documentation.is_empty());
        assert!(config.rules.is_empty());
    }

    #[test]
    fn test_parse_config_invalid_syntax() {
        let config_content = "trigger1 = > replacement1"; // Invalid syntax
        assert!(parse_textra_config(config_content).is_err());
    }

    #[test]
    fn test_serialize_deserialize_config_basic() {
        let original_config = TextraConfig {
            metadata: {
                let mut map = HashMap::new();
                map.insert("version".to_string(), "1.0".to_string());
                map
            },
            documentation: vec!["Test documentation.".to_string()],
            rules: vec![TextraRule {
                triggers: vec!["hi".to_string()],
                replacement: Replacement::Simple("hello".to_string()),
                description: Some("Greets".to_string()),
                category: Some("General".to_string()),
            }],
            overlay: OverlayConfig::default(),
        };

        let serialized_config = serialize_textra_config(&original_config);
        // println!("Serialized:\n{}", serialized_config); // For debugging
        
        // Note: Direct deserialization from this format is not perfectly symmetrical
        // because comments (description, category) are not parsed back into fields.
        // The parse_textra_config function would need to be enhanced to handle this.
        // For now, we test that it parses without error and some basic fields are correct.
        let deserialized_config = parse_textra_config(&serialized_config).unwrap();

        assert_eq!(deserialized_config.metadata.get("version"), Some(&"1.0".to_string()));
        assert_eq!(deserialized_config.documentation, vec!["Test documentation."]);
        assert_eq!(deserialized_config.rules.len(), 1);
        assert_eq!(deserialized_config.rules[0].triggers, vec!["hi"]);
        assert_eq!(deserialized_config.rules[0].replacement, Replacement::Simple("hello".to_string()));
        // Description and category are not parsed back by current parse_textra_config
        assert_eq!(deserialized_config.rules[0].description, None);
        assert_eq!(deserialized_config.rules[0].category, None);
    }

    #[test]
    fn test_propagate_case() {
        assert_eq!(replacement::propagate_case("test", "replacement"), "replacement");
        assert_eq!(replacement::propagate_case("Test", "replacement"), "Replacement");
        assert_eq!(replacement::propagate_case("TEST", "replacement"), "REPLACEMENT");
        assert_eq!(replacement::propagate_case("tEsT", "replacement"), "replacement"); // Mixed case trigger = lowercase replacement
        assert_eq!(replacement::propagate_case("", "replacement"), "replacement");
        assert_eq!(replacement::propagate_case("test", ""), "");
    }

    #[test]
    fn test_process_dynamic_replacement() {
        let date_regex = regex::Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
        let time_regex = regex::Regex::new(r"\d{2}:\d{2}:\d{2}").unwrap();

        let replaced_date = replacement::process_dynamic_replacement("Date: {{date}}");
        assert!(date_regex.is_match(&replaced_date));
        assert!(replaced_date.starts_with("Date: "));

        let replaced_time = replacement::process_dynamic_replacement("Time: {{time}}");
        assert!(time_regex.is_match(&replaced_time));
        assert!(replaced_time.starts_with("Time: "));

        let replaced_both = replacement::process_dynamic_replacement("Now: {{date}} {{time}}");
        assert!(date_regex.is_match(&replaced_both));
        assert!(time_regex.is_match(&replaced_both));
        assert!(replaced_both.starts_with("Now: "));

        assert_eq!(replacement::process_dynamic_replacement("Static text"), "Static text");
    }
}