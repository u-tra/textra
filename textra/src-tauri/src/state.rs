use super::*;
use anyhow::Result;
use chrono::Local;
use mouse::ClickType;
use notify::{RecursiveMode, Watcher};
use tauri::AppHandle;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use std::{mem, ptr};
// use view::Suggestion;
use winapi::ctypes::c_int;
use winapi::shared::minwindef::*;
use winapi::shared::windef::*;
use winapi::um::wingdi::*;
use winapi::um::{libloaderapi::GetModuleHandleW, winuser::*};

pub const MAX_TEXT_LENGTH: usize = 100;

pub struct AppState {
    pub config: Arc<Mutex<TextraConfig>>,
    pub current_text: Arc<Mutex<VecDeque<char>>>,
    pub last_key_time: Arc<Mutex<Instant>>,
    pub shift_pressed: Arc<AtomicBool>,
    pub ctrl_pressed: Arc<AtomicBool>,
    pub alt_pressed: Arc<AtomicBool>,
    pub caps_lock_on: Arc<AtomicBool>,
    pub killswitch: Arc<AtomicBool>,
    pub overlay_hwnd: Arc<Mutex<HWND>>,
    pub last_click_time: Arc<Mutex<Instant>>,
    pub last_click_x: AtomicI32,
    pub last_click_y: AtomicI32,
    pub click_count: AtomicI32,
  
}

impl AppState {
    pub fn new( ) -> Result<Self> {
        let config = load_config()?;

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            current_text: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_TEXT_LENGTH))),
            last_key_time: Arc::new(Mutex::new(Instant::now())),
            shift_pressed: Arc::new(AtomicBool::new(false)),
            ctrl_pressed: Arc::new(AtomicBool::new(false)),
            alt_pressed: Arc::new(AtomicBool::new(false)),
            caps_lock_on: Arc::new(AtomicBool::new(false)),
            killswitch: Arc::new(AtomicBool::new(false)),
            overlay_hwnd: Arc::new(Mutex::new(ptr::null_mut())),
            last_click_time: Arc::new(Mutex::new(Instant::now())),
            last_click_x: AtomicI32::new(0),
            last_click_y: AtomicI32::new(0),
            click_count: AtomicI32::new(0),
      
        })
    }

    pub fn get_overlay_hwnd(&self) -> HWND {
        self.overlay_hwnd.lock().unwrap().clone()
    }

    pub fn set_overlay_hwnd(&self, hwnd: HWND) {
        *self.overlay_hwnd.lock().unwrap() = hwnd;
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

    pub fn get_alt_pressed(&self) -> bool {
        self.alt_pressed.load(Ordering::SeqCst)
    }

    pub fn get_ctrl_pressed(&self) -> bool {
        self.ctrl_pressed.load(Ordering::SeqCst)
    }

    pub fn get_shift_pressed(&self) -> bool {
        self.shift_pressed.load(Ordering::SeqCst)
    }

    pub fn get_caps_lock_on(&self) -> bool {
        self.caps_lock_on.load(Ordering::SeqCst)
    }

    // pub fn get_suggestions(&self) -> Vec<Suggestion> {
    //     let config = self.config.lock().unwrap();
    //     let current_text: String = self.current_text.lock().unwrap().iter().collect();
    //     let suggestions = config.get_suggestions(&current_text);
    //     suggestions
    // }

    pub fn get_killswitch(&self) -> bool {
        self.killswitch.load(Ordering::SeqCst)
    }

    pub fn update_mouse_click(&self, x: i32, y: i32) {
        let mut last_click_time = self.last_click_time.lock().unwrap();
        let now = Instant::now();
        let time_since_last_click = now.duration_since(*last_click_time);
        let last_x = self.last_click_x.load(Ordering::Relaxed);
        let last_y = self.last_click_y.load(Ordering::Relaxed);

        if time_since_last_click <= Duration::from_millis(500) 
           && (x - last_x).abs() <= 4 
           && (y - last_y).abs() <= 4 {
            self.click_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.click_count.store(1, Ordering::Relaxed);
        }

        *last_click_time = now;
        self.last_click_x.store(x, Ordering::Relaxed);
        self.last_click_y.store(y, Ordering::Relaxed);
    }

    pub fn get_click_type(&self) -> ClickType {
        match self.click_count.load(Ordering::Relaxed) {
            1 => ClickType::Single,
            2 => ClickType::Double,
            _ => ClickType::Single,
        }
    }
}
