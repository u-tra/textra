#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(unused_imports, unused_variables, unused_mut, unused_assignments, unused_must_use)]

use anyhow::{Context, Result};
 
use installer::{handle_install, handle_uninstall, BANNER};
use minimo::showln;
 
use textra_lib::handle_run;
 
use tauri::{App, AppHandle, Manager, WebviewWindow};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use textra_lib::*;

const SERVICE_NAME: &str = "textra";

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let macos_launcher = tauri_plugin_autostart::MacosLauncher::LaunchAgent;
    let autostart_args = None;

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
         
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(macos_launcher, autostart_args))
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![greet])
        .setup(|app| {
            set_app_handle(app.handle().clone());
            
            Ok(())
        })
 
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

mod mouse;
use mouse::listen_mouse;

fn main() -> Result<()> {
 
    show_banner();
 
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "run" | "start" => {
                handle_run()?;
            }
            "stop" | "kill" => {
                handle_stop()?;
            }
            "install" | "setup" => {
                handle_install()?;
            }
            "uninstall" | "remove" => {
                handle_uninstall()?;
            }
            "edit" | "config" => {
                handle_edit_config()?;
            }
            _ => {
                show_help();
            }
        }
    } else {
        handle_run()?;    
    }

 

    Ok(())
}

fn show_banner() {
    showln!(white_bold, BANNER);
}

fn show_help() {
    show_banner();
    showln!(yellow_bold, "│ ", whitebg, " HOW TO USE ");
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
        "textra edit ",
        gray_dim,
        "- Edit the Textra configuration file"
    );
    showln!(yellow_bold, "│ ");
}

 