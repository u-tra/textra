//! Textra Command Line Interface
//! 
//! Provides CLI access to manage text expansion rules, processes, and configuration.

use anyhow::Context;
use std::{env, fs, process::Command, thread, time::Duration};
use std::os::windows::ffi::OsStrExt;
use std::ffi::OsStr;
use winapi::shared::minwindef::*;
use winapi::um::winbase::*;
use winapi::um::handleapi::*;
use winapi::um::libloaderapi::*;
use textra::{
    get_config_path, ipc, load_config, IpcMessage, Result, TextraConfig, TextraRule, Replacement,
    DAEMON_PIPE_NAME, OVERLAY_PIPE_NAME, process,
};
use tracing::{debug, error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

pub const BANNER: &str = r#"

  ██\                           ██\                        
  ██ |                          ██ |                       
██████\    ██████\  ██\   ██\ ██████\    ██████\  ██████\  
\_██  _|  ██  __██\ \██\ ██  |\_██  _|  ██  __██\ \____██\ 
  ██ |    ████████ | \████  /   ██ |    ██ |  \__|███████ |
  ██ |██\ ██   ____| ██  ██<    ██ |██\ ██ |     ██  __██ |
  \████  |\███████\ ██  /\██\   \████  |██ |     \███████ |
   \____/  \_______|\__/  \__|   \____/ \__|      \_______|
                                                
"#;

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
:tst => `twinkle twinkle little star, how i wonder what you are,up above the world so high,like a diamond in the sky`
ccc => continue writing complete code without skipping anything
"#;

// Check if the Textra service is running
fn is_service_running() -> bool {
    ipc::send_message(DAEMON_PIPE_NAME, &IpcMessage::StatusRequest).is_ok()
}

// Check if autostart is enabled
fn check_autostart() -> bool {
    // TODO: Implement registry check for autostart
    false
}

// Display colored status information
fn handle_display_status() {
    if is_service_running() {
        println!("│ service: \x1b[32mrunning\x1b[0m");
    } else {
        println!("│ service: \x1b[33mnot running\x1b[0m");
    }
    if check_autostart() {
        println!("│ autostart: \x1b[32menabled\x1b[0m");
    } else {
        println!("│ autostart: \x1b[33mdisabled\x1b[0m");
    }
}

// Display help menu with colored output
fn display_help() {
    println!("{}", BANNER.trim());
    println!("\x1b[33;1m┌─\x1b[47m STATUS \x1b[0m\x1b[33;1m──────────\x1b[0m");
    println!("\x1b[33;1m│\x1b[0m");
    handle_display_status();
    println!("\x1b[33;1m│\x1b[0m");
    println!("\x1b[33;1m│\x1b[47m HOW TO USE \x1b[0m");
    println!("\x1b[33;1m│\x1b[0m");
    println!("\x1b[33;1m│\x1b[36;1m textra run \x1b[90m- Start the Textra service\x1b[0m");
    println!("\x1b[33;1m│\x1b[36;1m textra stop \x1b[90m- Stop the running Textra service\x1b[0m");
    println!("\x1b[33;1m│\x1b[36;1m textra install \x1b[90m- Install Textra as a service\x1b[0m");
    println!("\x1b[33;1m│\x1b[36;1m textra uninstall \x1b[90m- Uninstall the Textra service\x1b[0m");
    println!("\x1b[33;1m│\x1b[36;1m textra edit \x1b[90m- Edit the Textra configuration file\x1b[0m");
    println!("\x1b[33;1m│\x1b[0m");
}

// Handle the run command
fn handle_run() -> Result<()> {
    if is_service_running() {
        println!("Textra is already running.");
        return Ok(());
    }

    // Start core daemon
    process::start_process_detached("textra-core.exe", &[])
        .context("Failed to start textra-core")?;

    // Small delay to let the daemon initialize
    thread::sleep(Duration::from_millis(100));

    // Start overlay
    process::start_process_detached("textra-overlay.exe", &[])
        .context("Failed to start textra-overlay")?;

    println!("Textra service started successfully.");
    Ok(())
}

// Handle the stop command
fn handle_stop() -> Result<()> {
    if !is_service_running() {
        println!("Textra is not running.");
        return Ok(());
    }

    // First stop the overlay process
    ipc::send_message(OVERLAY_PIPE_NAME, &IpcMessage::ShutdownOverlay)
        .context("Failed to send shutdown message to overlay")?;

    // Give overlay time to close
    thread::sleep(Duration::from_millis(100));

    // Then stop the daemon
    ipc::send_message(DAEMON_PIPE_NAME, &IpcMessage::StopDaemon)
        .context("Failed to send stop message to daemon")?;

    // Finally, ensure processes are terminated
    process::stop_process("textra-overlay.exe")?;
    process::stop_process("textra-core.exe")?;

    println!("Textra service stopped successfully.");
    Ok(())
}

// Handle the edit config command
fn handle_edit_config() -> Result<()> {
    let config_path = get_config_path()?;
    
    if !config_path.exists() {
        fs::write(&config_path, DEFAULT_CONFIG)
            .context("Failed to create default config file")?;
    }

    // Open config file in default editor
    #[cfg(target_os = "windows")]
    Command::new("notepad")
        .arg(&config_path)
        .spawn()
        .context("Failed to open config file in notepad")?;

    #[cfg(not(target_os = "windows"))]
    Command::new("xdg-open")
        .arg(&config_path)
        .spawn()
        .context("Failed to open config file")?;

    Ok(())
}

// Handle the install command
fn handle_install() -> Result<()> {
    // TODO: Implement service installation
    println!("Installing Textra service...");
    Ok(())
}

// Handle the uninstall command
fn handle_uninstall() -> Result<()> {
    // TODO: Implement service uninstallation
    println!("Uninstalling Textra service...");
    Ok(())
}

fn main() -> Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|e| {
                    eprintln!("Failed to parse RUST_LOG: {}. Using default 'info' level.", e);
                    EnvFilter::new("info")
                })
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set tracing subscriber")?;

    let args: Vec<String> = env::args().collect();

    // If no arguments provided, show help
    if args.len() == 1 {
        display_help();
        std::thread::sleep(std::time::Duration::from_secs(2));
        return Ok(());
    }

    // Handle commands
    match args[1].as_str() {
        "run" | "start" => handle_run(),
        "stop" | "kill" => handle_stop(),
        "install" | "setup" => handle_install(),
        "uninstall" | "remove" => handle_uninstall(),
        "config" | "edit" | "settings" => handle_edit_config(),
        _ => {
            display_help();
            Ok(())
        }
    }
}