use super::*;
use anyhow::Result;
use chrono::Local;
use notify::{RecursiveMode, Watcher};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use std::{mem, ptr};
use view::Suggestion;
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
}

impl AppState {
    pub fn new() -> Result<Self> {
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

    pub fn get_suggestions(&self) -> Vec<Suggestion> {
        let config = self.config.lock().unwrap();
        let current_text: String = self.current_text.lock().unwrap().iter().collect();
        let suggestions = config.get_suggestions(&current_text);
        suggestions
    }

    pub fn get_killswitch(&self) -> bool {
        self.killswitch.load(Ordering::SeqCst)
    }
}
