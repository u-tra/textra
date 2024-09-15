mod config;
pub mod keyboard;
pub mod parse;
pub mod installer;
pub use config::*;
pub use installer::*;
pub use keyboard::listen_keyboard;
pub use keyboard::main_loop;
use keyboard::AppState;
 
 
 pub use crossbeam_channel::*;
pub use ropey::Rope;
use std::sync::Arc;
pub use std::time::Instant;
pub use std::io;
pub use std::sync::atomic::AtomicBool;
pub use std::sync::atomic::AtomicUsize;
pub use std::sync::atomic::Ordering;
pub use std::sync::atomic::AtomicU64;
pub use std::sync::atomic::AtomicI64;
pub use std::sync::atomic::AtomicI32;
pub use std::sync::atomic::AtomicI16;


pub use anyhow::{Context, Result};
pub use chrono::Local;
pub use config::{Config, Message, Replacement, GLOBAL_SENDER};
pub use crossbeam_channel::{bounded, Receiver, Sender};
pub use dirs;
pub use minimo::banner::Banner;

pub use minimo::{
    cyan_bold, divider, divider_vibrant, gray_dim, green_bold, orange_bold, showln, white_bold,
    yellow_bold, Stylable,
};
 
pub use regex::Regex;
pub use ropey::*;
pub use serde::{Deserialize, Serialize};
pub use std::collections::HashMap;
pub use std::ffi::{c_int, OsString};
pub use std::io::Write;
pub use std::mem::MaybeUninit;
pub use std::os::windows::ffi::{OsStrExt, OsStringExt};
pub use std::os::windows::process::CommandExt;
pub use std::process::{exit, Command};
pub use std::time::{Duration};
pub use std::{env, fs, mem, ptr, thread};
pub use winapi::shared::minwindef::{DWORD, LPARAM, LRESULT, WPARAM};
pub use winapi::um::handleapi::*;
pub use winapi::um::minwinbase::STILL_ACTIVE;
pub use winapi::um::processthreadsapi::{GetExitCodeProcess, OpenProcess, TerminateProcess};
pub use winapi::um::synchapi::WaitForSingleObject;
pub use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};
pub use winapi::um::wincon::FreeConsole;

pub use winapi::um::winbase::*;
pub use winapi::um::winnt::{HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE};
pub use winapi::um::winuser::*;
pub use winreg::enums::*;
pub use winreg::RegKey;
pub use minimo::*;
const SERVICE_NAME: &str = "Textra";
const MUTEX_NAME: &str = "Global\\TextraRunning";



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
            showln!(green_bold, "Textra service started successfully");
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to start Textra service: {}", e));
        }
    }

    Ok(())
}

pub fn handle_daemon() -> Result<()> {
    //to avoid any kind of duplicate daemon we will only run if this is being executed from user_home/.textra/textra.exe
 
    // if is_running_from_install_dir() {
    //    showln!(orange_bold,"This is a standalone application and should not be run as a daemon. If you want to run Textra as a daemon, please use the 'textra run' command.");
    //    return Ok(());
    // }
    let app_state = Arc::new(AppState::new()?);
    let (sender, receiver) = bounded(100);

    let config_watcher = thread::spawn({
        let sender = sender.clone();
        move || config::watch_config(sender)
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

                if name.to_lowercase() == "textra.exe" && entry.th32ProcessID != current_pid as u32
                {
                    let process_handle =
                        OpenProcess(PROCESS_QUERY_INFORMATION, 0, entry.th32ProcessID);
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
