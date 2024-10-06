use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use std::thread;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use winapi::um::{winuser::*, libloaderapi::GetModuleHandleW};
use winapi::shared::windef::*;
use winapi::shared::minwindef::*;
use std::{ptr, mem};
use tauri::{App, AppHandle, Manager};
use chrono::Local;
use tokio::sync::Notify;
use anyhow::Result;

use crate::{tray, AppState};
 

const KEY_DELAY: u64 = 10;
const DOUBLE_PRESS_DELAY: u64 = 1000;
const MAX_TEXT_LENGTH: usize = 50;
static GENERATING: AtomicBool = AtomicBool::new(false);

 
pub static PIN: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));
pub static TMP_PIN: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));
pub static OLD: Lazy<Arc<RwLock<String>>> = Lazy::new(|| Arc::new(RwLock::new(String::new())));
pub static SIMULATION: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));


#[derive(Debug, Clone, Copy)]
pub enum Message {
    KeyEvent(DWORD, WPARAM, LPARAM),
    ConfigReload,
    Quit,
}

lazy_static::lazy_static!(
    pub static ref APP_HANDLE: Arc<Mutex<Option<AppHandle>>> = Arc::new(Mutex::new(None));
);

pub fn set_app_handle(handle: AppHandle) {
    *APP_HANDLE.lock().unwrap() = Some(handle);
}

pub fn get_app_handle() -> AppHandle {
  APP_HANDLE.lock().unwrap().as_ref().unwrap().clone()
}


pub fn panel( ) {

    tauri::WebviewWindowBuilder::new(&get_app_handle(), "panel", tauri::WebviewUrl::App("/".into()))
        .title("Tran")
        .inner_size(256.0, 100.0)
        .fullscreen(false)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .shadow(true)
        .center()
        .build()
        .expect("Failed to create panel window");
}
 
static mut GLOBAL_SENDER: Option<std::sync::mpsc::Sender<Message>> = None;
 