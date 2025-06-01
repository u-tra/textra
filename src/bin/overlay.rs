//! Textra Overlay UI
//!
//! This binary provides the visual overlay that appears when the user presses Shift twice.
//! It displays a selection of text expansion templates the user can choose from.

use std::{
    sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc},
    thread,
    time::Duration,
    collections::HashMap,
    fs,
    path::Path,
};

use anyhow::{Context, Result};
use serde::{Serialize, Deserialize};
use serde_json::Value as JsonValue; // For handling evaluate result
use web_view::{Content, WVResult, WebView, Error as WebViewError}; // Corrected Error import and alias
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber};
use parking_lot::Mutex;

use textra::{
    TextraConfig, TextraRule, Replacement, OverlayConfig,
    IpcMessage, load_config, ipc,
    OVERLAY_PIPE_NAME, DAEMON_PIPE_NAME,
    TextraError, get_default_html,
};

// Messages that can be sent to the webview thread
enum WebviewMessage {
    UpdateConfig(UIConfig),
    Show,
    Hide,
}

// UI State with thread-safe communication
struct OverlayState {
    config: Arc<Mutex<TextraConfig>>,
    visible: Arc<AtomicBool>,
    webview_tx: mpsc::Sender<WebviewMessage>,
    error_count: Arc<AtomicBool>,
    last_update: Arc<Mutex<Option<std::time::Instant>>>,
}

impl OverlayState {
    fn new(webview_tx: mpsc::Sender<WebviewMessage>) -> Result<Self> {
        let config = load_config()
            .context("Failed to load configuration for OverlayState")?;
        
        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            visible: Arc::new(AtomicBool::new(false)),
            webview_tx,
            error_count: Arc::new(AtomicBool::new(false)),
            last_update: Arc::new(Mutex::new(None)),
        })
    }
    
    fn update_config(&self, new_config: TextraConfig) -> Result<()> {
        *self.last_update.lock() = Some(std::time::Instant::now());
        *self.config.lock() = new_config.clone();
        
        let ui_config = self.get_ui_config()?;
        self.webview_tx.send(WebviewMessage::UpdateConfig(ui_config))
            .map_err(|e| anyhow::anyhow!("Failed to send config update: {}", e))?;
        
        Ok(())
    }
    
    fn show(&self) -> Result<()> {
        self.visible.store(true, Ordering::SeqCst);
        self.webview_tx.send(WebviewMessage::Show)
            .map_err(|e| anyhow::anyhow!("Failed to send show message: {}", e))?;
        Ok(())
    }
    
    fn hide(&self) -> Result<()> {
        self.visible.store(false, Ordering::SeqCst);
        self.webview_tx.send(WebviewMessage::Hide)
            .map_err(|e| anyhow::anyhow!("Failed to send hide message: {}", e))?;
        Ok(())
    }
    
    fn get_ui_config(&self) -> Result<UIConfig> {
        let config = self.config.lock().clone();
        
        let mut categories: HashMap<String, Vec<UIRule>> = HashMap::new();
        let mut uncategorized = Vec::new();
        
        for rule in &config.rules {
            let ui_rule = UIRule {
                triggers: rule.triggers.clone(),
                replacement: match &rule.replacement {
                    Replacement::Simple(text) => text.clone(),
                    Replacement::Multiline(text) => text.clone(),
                    Replacement::Code { language, content } => {
                        format!("Code ({}): {}", language, content.lines().next().unwrap_or(content))
                    }
                },
                description: rule.description.clone().unwrap_or_default(),
            };
            
            if let Some(category) = &rule.category {
                categories.entry(category.clone()).or_default().push(ui_rule);
            } else {
                uncategorized.push(ui_rule);
            }
        }
        
        let mut ui_categories: Vec<UICategory> = categories.into_iter()
            .map(|(name, rules)| UICategory { name, rules })
            .collect();
            
        ui_categories.sort_by(|a, b| a.name.cmp(&b.name));
        
        if !uncategorized.is_empty() {
            ui_categories.push(UICategory {
                name: "General".to_string(), // Default category name
                rules: uncategorized,
            });
        }
        
        Ok(UIConfig {
            categories: ui_categories,
            style: UIStyle {
                width: config.overlay.width,
                height: config.overlay.height,
                font_size: config.overlay.font_size,
                font_family: config.overlay.font_family.clone(),
                opacity: config.overlay.opacity,
                primary_color: config.overlay.primary_color.clone(),
                secondary_color: config.overlay.secondary_color.clone(),
                text_color: config.overlay.text_color.clone(),
                border_radius: config.overlay.border_radius,
            },
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct UIConfig {
    categories: Vec<UICategory>,
    style: UIStyle,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct UICategory {
    name: String,
    rules: Vec<UIRule>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct UIRule {
    triggers: Vec<String>,
    replacement: String,
    description: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct UIStyle {
    width: u32,
    height: u32,
    font_size: u32,
    font_family: String,
    opacity: f32,
    primary_color: String,
    secondary_color: String,
    text_color: String,
    border_radius: u32,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
enum UIEvent {
    TemplateSelected { text: String },
    CloseOverlay,
}

fn load_and_inline_html() -> Result<String> {
    let base_path = Path::new("assets/overlay");
    let html_path = base_path.join("index.html");
    let css_path = base_path.join("css/styles.css"); 
    let js_path = base_path.join("js/overlay.js");

    debug!("Attempting to load HTML from: {:?}", html_path);
    debug!("Attempting to load CSS from: {:?}", css_path);
    debug!("Attempting to load JS from: {:?}", js_path);

    let mut html_content = fs::read_to_string(&html_path)
        .with_context(|| format!("Failed to read HTML file: {:?}", html_path))?;
    
    let css_content = fs::read_to_string(&css_path)
        .with_context(|| format!("Failed to read CSS file: {:?}", css_path))?;
    
    let js_content = fs::read_to_string(&js_path)
        .with_context(|| format!("Failed to read JS file: {:?}", js_path))?;

    let css_link_tag = r#"<link rel="stylesheet" href="assets/overlay/css/styles.css">"#;
    let inline_css = format!("<style>\n/* Inlined from styles.css */\n{}\n</style>", css_content);
    if html_content.contains(css_link_tag) {
        html_content = html_content.replace(css_link_tag, &inline_css);
        debug!("Successfully inlined CSS.");
    } else {
        warn!("CSS link tag not found in index.html for inlining: '{}'. Attempting to inject into <head>.", css_link_tag);
        if let Some(head_close_pos) = html_content.rfind("</head>") { 
            html_content.insert_str(head_close_pos, &inline_css);
            debug!("CSS injected into <head> as fallback.");
        } else {
            warn!("</head> tag not found. Prepending CSS (less ideal).");
            html_content = format!("{}{}", inline_css, html_content);
        }
    }

    let js_script_tag = r#"<script src="assets/overlay/js/overlay.js"></script>"#;
    let inline_js = format!("<script>\n// Inlined from overlay.js\n{}\n</script>", js_content);
    if html_content.contains(js_script_tag) {
        html_content = html_content.replace(js_script_tag, &inline_js);
        debug!("Successfully inlined JS.");
    } else {
        warn!("JS script tag not found in index.html for inlining: '{}'. Attempting to inject before </body>.", js_script_tag);
         if let Some(body_close_pos) = html_content.rfind("</body>") { 
            html_content.insert_str(body_close_pos, &inline_js);
            debug!("JS injected before </body> as fallback.");
        } else {
            warn!("</body> tag not found. Appending JS (less ideal).");
            html_content.push_str(&inline_js);
        }
    }
    
    Ok(html_content)
}


fn run_webview(
    rx: mpsc::Receiver<WebviewMessage>,
    html: String,
    config: OverlayConfig,
    initial_config: UIConfig,
) -> WVResult<()> { 
    info!("Building webview...");
    let mut webview = web_view::builder()
        .title("Textra")
        .content(Content::Html(html))
        .size(config.width as i32, config.height as i32)
        .resizable(false)
        .debug(cfg!(debug_assertions)) 
        .user_data(())
        .invoke_handler(move |_wv, arg| {
            debug!("Invoke handler received: {}", arg);
            match serde_json::from_str::<UIEvent>(arg) {
                Ok(event) => {
                    match event {
                        UIEvent::TemplateSelected { text } => {
                            info!("Template selected via UI: {}", text);
                            if let Err(e) = ipc::send_message(
                                DAEMON_PIPE_NAME, 
                                &IpcMessage::TemplateSelected { text }
                            ) {
                                error!("Failed to send template: {}", e);
                            }
                        },
                        UIEvent::CloseOverlay => {
                            info!("CloseOverlay event received from UI.");
                            if let Err(e) = ipc::send_message(
                                DAEMON_PIPE_NAME,
                                &IpcMessage::HideOverlay 
                            ) {
                                error!("Failed to send HideOverlay message to daemon: {}", e);
                            }
                        }
                    }
                }
                Err(e) => error!("Failed to parse UI event: {}", e),
            }
            Ok(())
        })
        .build()?;
    info!("Webview built.");

    const MAX_READY_POLLS: usize = 60; 
    const POLL_INTERVAL_MS: u64 = 100;
    let mut ready_polls = 0;
    info!("Starting to poll for JavaScript readiness (isTextraAppReady)...");
    loop {
        let eval_script = "if (typeof window.isTextraAppReady !== 'function' || !window.isTextraAppReady()) { throw new Error('JS app not ready or isTextraAppReady() returned false'); }";
        match webview.eval(eval_script) {
            Ok(_) => {
                info!("JavaScript app (isTextraAppReady) is ready.");
                break; // Exit loop if the app is ready
            }
            Err(e) => {
                warn!("JavaScript app not ready yet (poll {}): {}", ready_polls, e);
            }
        }
     

        ready_polls += 1;
        if ready_polls >= MAX_READY_POLLS {
            error!("JavaScript app (isTextraAppReady) not ready after {} polls ({}ms timeout). Cannot initialize UI.", ready_polls, MAX_READY_POLLS * POLL_INTERVAL_MS as usize);
            return Err(WebViewError::custom("JS app not ready after timeout".to_string()));
        }
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
    info!("JavaScript readiness confirmed.");

    let init_config_json = serde_json::to_string(&initial_config)
        .map_err(|e| {
            error!("Failed to serialize initial UIConfig to JSON: {:?}", e);
            WebViewError::custom(e.to_string())
        })?;
    
    info!("Attempting to call window.textraApp.initConfig with JSON (first 100 chars): {}...", &init_config_json[..std::cmp::min(100, init_config_json.len())]);
    let init_script = format!(
        "try {{ console.log('[TextraOverlay] Rust: Attempting to call window.textraApp.initConfig...'); window.textraApp.initConfig({}); console.log('[TextraOverlay] Rust: initConfig call completed from JS perspective.'); }} catch (e) {{ console.error('[TextraOverlay] Rust: Error during initConfig call from Rust:', e, JSON.stringify(e)); throw e; }}",
        init_config_json
    );

    webview.eval(&init_script)?; 
    info!("initConfig script evaluated successfully using webview.eval().");

    info!("Attempting to call window.textraApp.hideOverlay() initially.");
    webview.eval("window.textraApp.hideOverlay()")?;


    #[cfg(target_os = "windows")]
    {
        info!("Applying Windows-specific styles (transparency).");
        use winapi::um::winuser::*;
        
        unsafe {
            let hwnd = webview.window_handle() as winapi::shared::windef::HWND;
            if !hwnd.is_null() {
                let ex_style = GetWindowLongA(hwnd, GWL_EXSTYLE);
                SetWindowLongA(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as i32 | WS_EX_TOOLWINDOW as i32);
                
                SetLayeredWindowAttributes(
                    hwnd,
                    0, 
                    (255.0 * config.opacity) as u8,
                    LWA_ALPHA,
                );
                info!("Windows-specific styles applied.");
            } else {
                warn!("Could not get HWND for Windows-specific styles.");
            }
        }
    }

    let handle = webview.handle();
    info!("Starting webview message handling loop...");
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            let current_handle = handle.clone(); 
            match msg {
                WebviewMessage::UpdateConfig(config_data) => {
                    info!("Received UpdateConfig message for webview.");
                    if let Ok(json_data) = serde_json::to_string(&config_data) {
                        debug!("Dispatching initConfig to webview with new data (first 100 chars): {}...", &json_data[..std::cmp::min(100, json_data.len())]);
                        if let Err(e) = current_handle.dispatch(move |wv_instance| {
                            wv_instance.eval(&format!("window.textraApp.initConfig({})", json_data))
                        }) {
                            error!("Failed to dispatch initConfig to webview: {:?}", e);
                        }
                    } else {
                        error!("Failed to serialize UIConfig for webview update.");
                    }
                }
                WebviewMessage::Show => {
                    info!("Received Show message for webview.");
                    if let Err(e) = current_handle.dispatch(|wv_instance| {
                        #[cfg(target_os = "windows")]
                        unsafe {
                            let hwnd = wv_instance.window_handle() as winapi::shared::windef::HWND;
                            if !hwnd.is_null() {
                                winapi::um::winuser::ShowWindow(hwnd, winapi::um::winuser::SW_SHOW);
                                winapi::um::winuser::SetForegroundWindow(hwnd);
                            }
                        }
                        wv_instance.eval("window.textraApp.showOverlay()")
                    }) {
                        error!("Failed to dispatch showOverlay to webview: {:?}", e);
                    }
                }
                WebviewMessage::Hide => {
                    info!("Received Hide message for webview.");
                    if let Err(e) = current_handle.dispatch(|wv_instance| {
                        wv_instance.eval("window.textraApp.hideOverlay()")
                    }) {
                        error!("Failed to dispatch hideOverlay to webview: {:?}", e);
                    }
                }
            }
        }
        info!("Webview message receiver thread finished.");
    });

    info!("Running webview...");
    webview.run()
}

fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|e| {
                    eprintln!("Failed to parse RUST_LOG: {}. Using default 'info,web_view=info' level.", e);
                    EnvFilter::new("info,web_view=info") 
                })
        )
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_target(true) 
        .with_level(true)
        .with_ansi(true) 
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set tracing subscriber")?;

    info!("Textra Overlay starting...");

    let (tx, rx) = mpsc::channel();
    let state = Arc::new(OverlayState::new(tx)?);
    
    let state_ipc = Arc::clone(&state);
    ipc::listen(OVERLAY_PIPE_NAME, move |message| {
        let state_cb = Arc::clone(&state_ipc);
        debug!("Overlay IPC received: {:?}", message);
        match message {
            IpcMessage::ShowOverlay | IpcMessage::ShiftShiftDetected => {
                if let Err(e) = state_cb.show() {
                    error!("Failed to show overlay: {}", e);
                    state_cb.error_count.store(true, Ordering::SeqCst);
                }
            },
            IpcMessage::HideOverlay => {
                if let Err(e) = state_cb.hide() {
                    error!("Failed to hide overlay: {}", e);
                    state_cb.error_count.store(true, Ordering::SeqCst);
                }
            },
            IpcMessage::ConfigReloaded { config } => {
                info!("ConfigReloaded IPC received by overlay.");
                if let Err(e) = state_cb.update_config(config) {
                    error!("Failed to update config: {}", e);
                    state_cb.error_count.store(true, Ordering::SeqCst);
                }
            },
            IpcMessage::ShutdownOverlay => {
                info!("ShutdownOverlay IPC received. Exiting overlay process.");
                std::process::exit(0);
            }
            _ => debug!("Overlay ignoring IPC message: {:?}", message),
        }
        
        Ok(())
    })?;
    info!("Overlay IPC listener started on pipe: {}", OVERLAY_PIPE_NAME);

    let ui_config = state.get_ui_config()?;
    let overlay_config = state.config.lock().clone().overlay;
    debug!("Initial UIConfig prepared: {:?}", ui_config);

    let html = load_and_inline_html().unwrap_or_else(|e| {
        error!("Failed to load and inline HTML assets: {}. Using default HTML from lib.rs.", e);
        get_default_html()
    });

    info!("Spawning webview thread...");
    let webview_thread = thread::spawn(move || {
        info!("Webview thread started. Calling run_webview.");
        match run_webview(rx, html, overlay_config, ui_config) {
            Ok(_) => info!("run_webview finished successfully."),
            Err(e) => error!("run_webview failed: {:?}", e),
        }
    });

    info!("Overlay main thread entering monitoring loop.");
    webview_thread.join().unwrap_or_else(|e| {
        error!("Webview thread panicked: {:?}", e);
    });

    info!("Textra overlay shutting down after webview thread completion.");
    Ok(())
}