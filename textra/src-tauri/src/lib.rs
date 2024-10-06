#![allow(unused_imports, unused_variables, unused_mut, unused_assignments, unused_must_use, unused)]
use tauri::{App, Emitter, Manager, WebviewWindow};
 

pub mod config;
pub mod installer;
pub mod keyboard;
pub mod state;
pub mod parser;
pub mod setup;
pub mod tray;
 
pub mod mouse;
 

pub use crate::state::*;
pub use crate::parser::*;
pub use crate::config::*;
pub use crate::keyboard::*;
pub use crate::setup::*;
pub use crate::tray::*;
 
pub use crate::mouse::*;



#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

use tauri::{ PhysicalPosition, LogicalSize};
use parking_lot::{RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::state::*;
use crate::mouse::*;
use anyhow::Result;
use crate::setup::*;
use tauri::{AppHandle};


#[derive(Debug, Clone, serde::Serialize)]
pub struct Suggestions {
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Suggestion {
    pub label: String,
    pub value: String,
}


pub fn show( content: String) -> Result<()> {
   
    let panel = get_app_handle().get_webview_window("panel").expect("Failed to get panel window");
    tauri::async_runtime::spawn(async move {
        let result = build_suggestions(&content).unwrap();
      get_app_handle()
            .emit::<Suggestions>("show", result.into())
            .expect("Failed to emit show event");
    });

    
    
    if !PIN.load(Ordering::SeqCst) {
        let (x,y) = mouse::get_mouse_position();       
        panel.set_position(PhysicalPosition { x, y })?;
        panel.set_size(LogicalSize {
            width: 256,
            height: 100,
        })?;
       
        panel.show()?;
        panel.set_focus()?;
    }

    Ok(())
}


pub fn build_suggestions(content: &str) -> Result<Suggestions> {
    let suggestions = vec![
        Suggestion { label: "Hello".to_string(), value: "World".to_string() },
        Suggestion { label: "Foo".to_string(), value: "Bar".to_string() },
    ];
    Ok(Suggestions { suggestions })
}



 


use anyhow::{Context};
use chrono::Local;
use std::sync::mpsc::{channel, Receiver, Sender};
use dirs;
use minimo::{
    banner::Banner,
    cyan_bold, divider, divider_vibrant, gray_dim, green_bold, orange_bold, red_bold, showln, white_bold,
    yellow_bold, Stylable,
};
use regex::Regex;
use ropey::Rope;
use std::{
    env, fs, io, mem, ptr, thread,
    time::{Duration, Instant},
 
    collections::HashMap,
    ffi::{c_int, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    os::windows::process::CommandExt,
    process::{exit, Command},
};
use winapi::{
    shared::minwindef::{DWORD, LPARAM, LRESULT, WPARAM},
    um::{
        handleapi::*, minwinbase::STILL_ACTIVE,
        processthreadsapi::{GetExitCodeProcess, OpenProcess, TerminateProcess},
        synchapi::WaitForSingleObject,
        tlhelp32::{CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS},
        wincon::FreeConsole,
        winbase::*, winnt::{HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE},
        winuser::*,
    },
};
use winreg::{enums::*, RegKey};

 


const SERVICE_NAME: &str = "Textra";
const MUTEX_NAME: &str = "Global\\TextraRunning";

#[derive(Debug, Clone, serde::Serialize)]
pub enum AppMessage {
    Quit,
    ShowPanel,
    ConfigChanged,
    KeyEvent(DWORD, WPARAM, LPARAM),
}

pub fn handle_run() -> Result<()> {
    if is_service_running() {
        showln!(yellow_bold, "textra is already running.");
        return Ok(());
    }
    let mut command = std::process::Command::new(env::current_exe()?);
    command.arg("daemon");
    command.creation_flags(winapi::um::winbase::DETACHED_PROCESS);
    match command.spawn() {
        Ok(_) => {
            showln!(gray_dim, "textra service ", green_bold, "started.");
         
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to start Textra service: {}", e));
        }
    }

    Ok(())
}

pub fn handle_daemon() -> Result<()> {

    let app_state = Arc::new(AppState::new().context("Failed to create AppState")?);
    let (sender, receiver) = channel::<AppMessage>();

    let config_watcher = thread::spawn({
        let sender = sender.clone();
        move || watch_config(sender).map_err(|e| anyhow::anyhow!("Config watcher error: {}", e))
    });

    let keyboard_listener = thread::spawn({
        let sender = sender.clone();
        move || listen_keyboard().map_err(|e| anyhow::anyhow!("Keyboard listener error: {}", e))
    });

    let mouse_listener = thread::spawn({

        move || mouse::listen_mouse().map_err(|e| anyhow::anyhow!("Mouse listener error: {}", e))
    });

    match main_loop(app_state, &receiver) {
        Ok(_) => {
            sender.send(AppMessage::Quit).unwrap();
            config_watcher.join().unwrap().context("Config watcher thread panicked")?;
            keyboard_listener.join().unwrap().context("Keyboard listener thread panicked")?;
            mouse_listener.join().unwrap().context("Mouse listener thread panicked")?;
        }
        Err(e) => {
            sender.send(AppMessage::Quit).unwrap();
            config_watcher.join().unwrap().context("Config watcher thread panicked")?;
            keyboard_listener.join().unwrap().context("Keyboard listener thread panicked")?;
            mouse_listener.join().unwrap().context("Mouse listener thread panicked")?;
            return Err(e);
        }
    }

    Ok(())
}

pub fn handle_stop() -> Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(anyhow::anyhow!("Failed to create process snapshot"));
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

pub fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "run" => handle_run()?,
            "stop" => handle_stop()?,
            "daemon" => handle_daemon()?,
            "edit" => handle_edit_config()?,
            "config" => display_config(),
            _ => {
                showln!(orange_bold, "Invalid command. Use 'run', 'stop', 'edit', or 'config'.");
            }
        }
    } else {
        showln!(orange_bold, "Please specify a command: 'run', 'stop', 'edit', or 'config'.");
    }

    Ok(())
}