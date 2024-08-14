use anyhow::{Context, Result};
use chrono::Local;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};
use tokio::task;
use dirs;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use rdev::{listen, simulate, Event, EventType, Key};
use regex::Regex;
use serde::{Deserialize, Serialize};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{env, fs};

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct Config {
    matches: Vec<Match>,
    #[serde(default)]
    backend: BackendConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Match {
    trigger: String,
    replace: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    propagate_case: bool,
    #[serde(default)]
    word: bool,
    #[serde(default)]
    dynamic: bool,
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
    current_text: Arc<Mutex<String>>,
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
            current_text: Arc::new(Mutex::new(String::new())),
            last_key_time: Arc::new(Mutex::new(Instant::now())),
            shift_pressed: Arc::new(AtomicBool::new(false)),
            caps_lock_on: Arc::new(AtomicBool::new(false)),
            killswitch: Arc::new(AtomicBool::new(false)),
        })
    }
}

enum Message {
    KeyEvent(Event),
    ConfigReload,
    Quit,
}

#[tokio::main]
async fn main() -> Result<()> {
    let app_state = Arc::new(AppState::new()?);
    let (sender, mut receiver) = mpsc::channel(100);

    let config_watcher = task::spawn(watch_config(sender.clone()));
    let keyboard_listener = task::spawn(listen_keyboard(sender.clone()));

    main_loop(app_state, &mut receiver).await?;

    config_watcher.abort();
    keyboard_listener.abort();

    Ok(())
}

async fn main_loop(app_state: Arc<AppState>, receiver: &mut mpsc::Receiver<Message>) -> Result<()> {
    while let Some(msg) = receiver.recv().await {
        match msg {
            Message::KeyEvent(event) => {
                if let Err(e) = handle_key_event(Arc::clone(&app_state), event).await {
                    eprintln!("Error handling key event: {}", e);
                }
            }
            Message::ConfigReload => {
                if let Err(e) = reload_config(Arc::clone(&app_state)).await {
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

    minimo::showln!(yellow_bold, "┌─ ", white, "TEXTRA ", yellow_bold,"───────────────────────────────────────────────────────");
    minimo::showln!(yellow_bold, "│ ", green_bold,config_path.display());
    if !config.matches.is_empty() {
        
        for match_rule in &config.matches {
            let trim_length = if match_rule.replace.len() + 3 > 50 { 50 } else { match_rule.replace.len() + 3 };
            let trimmed_replace = minimo::chop(&match_rule.replace ,trim_length)[0].clone();
            minimo::showln!(yellow_bold, "│ ",yellow_bold,"▫ ", gray_dim, match_rule.trigger, cyan_bold, " ⋯→ ", white_bold, trimmed_replace);
        }
    }
    // width is 60 characters
    minimo::showln!(yellow_bold, "└────────────────────────────────────────────────────────────────");
    minimo::showln!(gray_dim, "");
    Ok(config)
}

fn get_config_path() -> Result<PathBuf> {
    let current_dir = env::current_dir()?;
    let current_dir_config = current_dir.join("config.yaml");
    if current_dir_config.exists() {
        return Ok(current_dir_config);
    }

    if let Some(home_dir) = dirs::home_dir() {
        let home_config_dir = home_dir.join(".textra");
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
        matches: vec![],
        backend: BackendConfig { key_delay: 10 },
    };
    let yaml = serde_yaml::to_string(&default_config)?;
    fs::write(path, yaml).context("Failed to write default config file")?;
    Ok(())
}

async fn watch_config(sender: mpsc::Sender<Message>) -> Result<()> {
    let config_path = get_config_path()?;
    let (tx, mut rx) = mpsc::channel(100);

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if event.kind.is_modify() {
                let _ = tx.blocking_send(());
            }
        }
    })?;

    watcher.watch(config_path.parent().unwrap(), RecursiveMode::NonRecursive)?;

    while rx.recv().await.is_some() {
        sender.send(Message::ConfigReload).await?;
    }

    Ok(())
}

async fn listen_keyboard(sender: mpsc::Sender<Message>) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(100);

    task::spawn_blocking(move || {
        let callback = move |event: Event| {
            let _ = tx.blocking_send(event);
        };
        if let Err(error) = listen(callback) {
            eprintln!("Error: {:?}", error);
        }
    });

    while let Some(event) = rx.recv().await {
        sender.send(Message::KeyEvent(event)).await?;
    }

    Ok(())
}

async fn handle_key_event(app_state: Arc<AppState>, event: Event) -> Result<()> {
    let now = Instant::now();

    match event.event_type {
        EventType::KeyPress(key) => {
            let mut last_key_time = app_state.last_key_time.lock();
            if now.duration_since(*last_key_time) > Duration::from_millis(500) {
                app_state.current_text.lock().clear();
            }
            *last_key_time = now;

            match key {
                Key::Escape => {
                    app_state.killswitch.store(true, Ordering::SeqCst);
                }
                Key::ShiftLeft | Key::ShiftRight => {
                    app_state.shift_pressed.store(true, Ordering::SeqCst);
                }
                Key::CapsLock => {
                    let current = app_state.caps_lock_on.load(Ordering::SeqCst);
                    app_state.caps_lock_on.store(!current, Ordering::SeqCst);
                }
                _ => {
                    if let Some(c) = key_to_char(
                        key,
                        app_state.shift_pressed.load(Ordering::SeqCst),
                        app_state.caps_lock_on.load(Ordering::SeqCst),
                    ) {
                        let mut current_text = app_state.current_text.lock();
                        current_text.push(c);
                        check_and_replace(&app_state, &mut current_text).await?;
                    }
                }
            }
        }
        EventType::KeyRelease(key) => match key {
            Key::ShiftLeft | Key::ShiftRight => {
                app_state.shift_pressed.store(false, Ordering::SeqCst);
            }
            Key::Escape => {
                app_state.killswitch.store(false, Ordering::SeqCst);
            }
            _ => {}
        },
        _ => {}
    }

    Ok(())
}

async fn check_and_replace(app_state: &AppState, current_text: &mut String) -> Result<()> {
    let immutable_current_text = current_text.clone();
    let config = app_state.config.lock();
    for match_rule in &config.matches {
        if match_rule.regex {
            let regex = Regex::new(&match_rule.trigger)?;
            if let Some(captures) = regex.captures(&immutable_current_text) {
                let mut replacement = match_rule.replace.clone();
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
                    match_rule.propagate_case,
                    match_rule.dynamic,
                    app_state,
                ).await?;
                break;
            }
        } else if current_text.ends_with(&match_rule.trigger) {
            let start = immutable_current_text.len() - match_rule.trigger.len();
            if !match_rule.word
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
                    &match_rule.replace,
                    match_rule.propagate_case,
                    match_rule.dynamic,
                    app_state,
                ).await?;
                break;
            }
        }
    }
    Ok(())
}

async fn perform_replacement(
    current_text: &mut String,
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

    let duration = Duration::from_millis(key_delay);

    // Backspace the original text
    for _ in 0..original.len() {
        if app_state.killswitch.load(Ordering::SeqCst) {
            return Ok(());
        }
        simulate(&EventType::KeyPress(Key::Backspace))?;
        simulate(&EventType::KeyRelease(Key::Backspace))?;
        sleep(duration).await;
    }

    // Type the replacement
    for c in final_replacement.chars() {
        if app_state.killswitch.load(Ordering::SeqCst) {
            return Ok(());
        }   
        let key = char_to_key(c);
        simulate(&EventType::KeyPress(key))?;
        simulate(&EventType::KeyRelease(key))?;
        sleep(duration).await;
    }

    *current_text = current_text[..current_text.len() - original.len()].to_string() + &final_replacement;
    Ok(())
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

async fn reload_config(app_state: Arc<AppState>) -> Result<()> {
    let mut config = app_state.config.lock();
    *config = load_config()?;
    Ok(())
}

fn key_to_char(key: Key, shift_pressed: bool, caps_lock_on: bool) -> Option<char> {
    let base_char = match key {
        Key::KeyA => 'a', Key::KeyB => 'b', Key::KeyC => 'c', Key::KeyD => 'd',
        Key::KeyE => 'e', Key::KeyF => 'f', Key::KeyG => 'g', Key::KeyH => 'h',
        Key::KeyI => 'i', Key::KeyJ => 'j', Key::KeyK => 'k', Key::KeyL => 'l',Key::KeyM => 'm', Key::KeyN => 'n', Key::KeyO => 'o', Key::KeyP => 'p',
        Key::KeyQ => 'q', Key::KeyR => 'r', Key::KeyS => 's', Key::KeyT => 't',
        Key::KeyU => 'u', Key::KeyV => 'v', Key::KeyW => 'w', Key::KeyX => 'x',
        Key::KeyY => 'y', Key::KeyZ => 'z',
        Key::Num0 => '0', Key::Num1 => '1', Key::Num2 => '2', Key::Num3 => '3',
        Key::Num4 => '4', Key::Num5 => '5', Key::Num6 => '6', Key::Num7 => '7',
        Key::Num8 => '8', Key::Num9 => '9',
        Key::Space => ' ', Key::Comma => ',', Key::SemiColon => ';', Key::Dot => '.',
        Key::Slash => '/', Key::Quote => '\'', Key::LeftBracket => '[',
        Key::RightBracket => ']', Key::BackSlash => '\\', Key::Minus => '-',
        Key::Equal => '=',
        _ => return None,
    };

    let shift_char = match key {
        Key::Num0 => ')', Key::Num1 => '!', Key::Num2 => '@', Key::Num3 => '#',
        Key::Num4 => '$', Key::Num5 => '%', Key::Num6 => '^', Key::Num7 => '&',
        Key::Num8 => '*', Key::Num9 => '(',
        Key::Comma => '<', Key::SemiColon => ':', Key::Dot => '>', Key::Slash => '?',
        Key::Quote => '"', Key::LeftBracket => '{', Key::RightBracket => '}',
        Key::BackSlash => '|', Key::Minus => '_', Key::Equal => '+',
        _ => base_char.to_ascii_uppercase(),
    };

    let final_char = if shift_pressed ^ caps_lock_on {
        shift_char
    } else {
        base_char
    };

    Some(final_char)
}

fn char_to_key(c: char) -> Key {
    match c.to_ascii_lowercase() {
        'a' => Key::KeyA, 'b' => Key::KeyB, 'c' => Key::KeyC, 'd' => Key::KeyD,
        'e' => Key::KeyE, 'f' => Key::KeyF, 'g' => Key::KeyG, 'h' => Key::KeyH,
        'i' => Key::KeyI, 'j' => Key::KeyJ, 'k' => Key::KeyK, 'l' => Key::KeyL,
        'm' => Key::KeyM, 'n' => Key::KeyN, 'o' => Key::KeyO, 'p' => Key::KeyP,
        'q' => Key::KeyQ, 'r' => Key::KeyR, 's' => Key::KeyS, 't' => Key::KeyT,
        'u' => Key::KeyU, 'v' => Key::KeyV, 'w' => Key::KeyW, 'x' => Key::KeyX,
        'y' => Key::KeyY, 'z' => Key::KeyZ,
        '0' => Key::Num0, '1' => Key::Num1, '2' => Key::Num2, '3' => Key::Num3,
        '4' => Key::Num4, '5' => Key::Num5, '6' => Key::Num6, '7' => Key::Num7,
        '8' => Key::Num8, '9' => Key::Num9,
        ' ' => Key::Space, ',' => Key::Comma, '.' => Key::Dot, '/' => Key::Slash,
        ';' => Key::SemiColon, '\'' => Key::Quote, '[' => Key::LeftBracket,
        ']' => Key::RightBracket, '\\' => Key::BackSlash, '-' => Key::Minus,
        '=' => Key::Equal,
        _ => Key::Space,
    }
}