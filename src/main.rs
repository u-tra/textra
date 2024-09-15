#![allow(unused_imports, unused_variables, unused_mut, unused_assignments, unused_imports)]

use anyhow::{Context, Result};
use chrono::Local;
use config::{Config, Message, Replacement, GLOBAL_SENDER};
use crossbeam_channel::{bounded, Receiver, Sender};
use dirs;
use minimo::banner::Banner;

use minimo::{cyan_bold, divider, divider_vibrant, gray_dim, green_bold, orange_bold, showln, white_bold, yellow_bold, Stylable};
use parking_lot::Mutex;
use regex::Regex;
 use ropey::*;
use serde::{Deserialize, Serialize};
use winapi::shared::minwindef::{DWORD, LPARAM, LRESULT, WPARAM};
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::wincon::FreeConsole;
use std::collections::HashMap;
use std::ffi::{c_int, OsString};
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::process::{exit, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, fs, io, mem, ptr, thread};
use winapi::um::handleapi::*;
use winapi::um::processthreadsapi::{GetExitCodeProcess, OpenProcess, TerminateProcess};
use winapi::um::synchapi::WaitForSingleObject;
use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};

use winapi::um::winbase::*;
use winapi::um::winnt::{HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE};
use winapi::um::winuser::*;
use winreg::enums::*;
use winreg::RegKey;

const SERVICE_NAME: &str = "Textra";
const MUTEX_NAME: &str = "Global\\TextraRunning";

mod config;
use config::*;
use installer::*;
mod installer;
pub mod keyboard;
use keyboard::*;


fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() == 1 {
        return display_help();
    }

    match args[1].as_str() {
        "run" | "start" => handle_run(),
        "config" | "edit" | "settings" => handle_edit_config(),
        "daemon" | "service" => handle_daemon(),
        "stop" | "kill" => handle_stop(),
        "install" | "setup" => installer::install(),
        "uninstall" | "remove" => installer::uninstall(),
        _ => display_help(),
    }
}

fn handle_run() -> Result<()> {
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

fn handle_daemon() -> Result<()> {
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

fn handle_stop() -> Result<()> {
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
                            showln!(green_bold, "Textra service stopped successfully.");
                        } else {
                            showln!(orange_bold, "Failed to terminate Textra process.");
                        }
                        CloseHandle(process_handle);
                    } else {
                        showln!(orange_bold, "Failed to open Textra process.");
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
        showln!(orange_bold, "Textra service is not running.");
    }

    Ok(())
}

fn is_service_running() -> bool {
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

fn handle_display_status() -> Result<()> {
    if is_service_running() {
        showln!(yellow_bold, "│ ", gray_dim, "service: ", green_bold, "running.");
    } else {
        showln!(yellow_bold, "│ ", gray_dim, "service: ", orange_bold, "not running.");
    }
    if installer::check_autostart() {
        showln!(yellow_bold, "│ ", gray_dim, "autostart: ", green_bold, "enabled.");
    } else {
        showln!(yellow_bold, "│ ", gray_dim, "autostart: ", orange_bold, "disabled.");
    }
    Ok(())
}


fn display_help() -> Result<()> {
     BANNER.show(white_bold);
     divider();
    showln!(
        yellow_bold,
        "┌─ ",
        whitebg,
        " STATUS ",
        yellow_bold,
        " ──────────"
    );
    showln!(yellow_bold, "│ ");
    handle_display_status()?;
    showln!(yellow_bold, "│ ");
    showln!(yellow_bold, "│ ",  whitebg, " HOW TO USE " );
    showln!(yellow_bold, "│ ");
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra run ",
        gray_dim,
        "- Start the Textra service"
    );
    showln!(
            yellow_bold,
        "│ ",
        cyan_bold,
        "textra stop ",
        gray_dim,
        "- Stop the running Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra install ",
        gray_dim,
        "- Install Textra as a service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra uninstall ",
        gray_dim,
        "- Uninstall the Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra status ",
        gray_dim,
        "- Display the status of the Textra service"
    );
    showln!(
        yellow_bold,
        "│ ",
        cyan_bold,
        "textra edit ",
        gray_dim,
        "- Edit the Textra configuration file"
    );
    showln!(
        yellow_bold,
        "│ " 
    );
 
    config::display_config();
    Ok(())
}



