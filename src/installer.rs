use std::env;
use std::fs;
use std::path::PathBuf;
use std::ptr;
use minimo::showln;
use winapi::um::winuser::{SendMessageTimeoutA, HWND_BROADCAST, WM_SETTINGCHANGE};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;
use anyhow::{Context, Result};

use crate::handle_stop;
use crate::is_service_running;


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
const SERVICE_NAME: &str = "textra";
const AUTO_START_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALLER_CODE: &str = r#"
    @echo off
    taskkill /F /IM textra.exe
    rmdir /S /Q "%LOCALAPPDATA%\Textra"
    reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v Textra /f
    echo Textra has been uninstalled.
"#;

pub fn install() -> Result<()> {
    showln!(yellow_bold, "Installing Textra...");
    
    if is_service_running() {
        showln!(orange_bold, "Detected already running instance, stopping it...");
        handle_stop().context("Failed to stop running instance")?;
    }

    let exe_path = env::current_exe().context("Failed to get current executable path")?;
    let install_dir = get_install_dir()?;
    fs::create_dir_all(&install_dir).context("Failed to create installation directory")?;
    let install_path = install_dir.join("textra.exe");

    fs::copy(&exe_path, &install_path).context("Failed to copy executable to install directory")?;

    add_to_path(&install_dir).context("Failed to add Textra to PATH")?;
    set_autostart(&install_path).context("Failed to set autostart")?;
    create_uninstaller(&install_dir).context("Failed to create uninstaller")?;

    showln!(green_bold, "Textra has been successfully installed");
    showln!(
        gray_dim,
        "To uninstall Textra, run ",
        yellow_bold,
        "textra uninstall",
        gray_dim,
        " in the terminal"
    );
    Ok(())
}

pub fn uninstall() -> Result<()> {
    showln!(yellow_bold, "Uninstalling Textra...");
    
    handle_stop().context("Failed to stop running instance")?;
    remove_from_path().context("Failed to remove Textra from PATH")?;
    remove_autostart().context("Failed to remove autostart entry")?;

    let install_dir = get_install_dir()?;
    fs::remove_dir_all(&install_dir).context("Failed to remove installation directory")?;

    showln!(green_bold, "Textra has been successfully uninstalled");
    Ok(())
}

fn get_install_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|dir| dir.join("Textra"))
        .context("Failed to determine local data directory")
}

fn add_to_path(install_dir: &std::path::Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu.create_subkey("Environment").context("Failed to open Environment registry key")?;
    
    let current_path: String = env.get_value("PATH").context("Failed to get current PATH")?;
    let new_path = if !current_path.contains(&install_dir.to_string_lossy().to_string()) {
        format!("{};{}", current_path, install_dir.to_string_lossy())
    } else {
        current_path
    };
    
    env.set_value("PATH", &new_path).context("Failed to set new PATH")?;

    update_environment_message();
    
    showln!(
        gray_dim,
        "Added ",
        yellow_bold,
        "Textra",
        gray_dim,
        " to the ",
        green_bold,
        "PATH",
        gray_dim,
        " environment variable."
    );
    Ok(())
}

fn set_autostart(install_path: &std::path::Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(AUTO_START_PATH).context("Failed to open Run registry key")?;
    let command = format!(
        r#"cmd /C start /min "" "{}" run"#,
        install_path.to_string_lossy()
    );
    key.set_value("Textra", &command).context("Failed to set autostart registry value")?;
    
    showln!(
        gray_dim,
        "Registered ",
        yellow_bold,
        "textra ",
        gray_dim,
        "for ",
        green_bold,
        "autostart",
        gray_dim,
        " in the registry."
    );
    Ok(())
}

pub fn check_autostart() -> bool{
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey(AUTO_START_PATH) {
        if let Ok(value) = key.get_value::<String, String>("Textra".to_string()) {
           if !value.is_empty() {
            return true;
           }
        }
    }
    false
}

fn remove_from_path() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu.create_subkey("Environment").context("Failed to open Environment registry key")?;
    
    let current_path: String = env.get_value("PATH").context("Failed to get current PATH")?;
    let install_dir = get_install_dir()?;
    let new_path: Vec<&str> = current_path
        .split(';')
        .filter(|&p| p != install_dir.to_str().unwrap())
        .collect();
    let new_path = new_path.join(";");
    
    env.set_value("PATH", &new_path).context("Failed to set new PATH")?;

    update_environment_message();
    
    showln!(
        gray_dim,
        "Removed ",
        yellow_bold,
        "Textra",
        gray_dim,
        " from the ",
        green_bold,
        "PATH",
        gray_dim,
        " environment variable."
    );
    Ok(())
}

fn remove_autostart() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(AUTO_START_PATH).context("Failed to open Run registry key")?;
    
    if let Err(e) = key.delete_value("Textra") {
        showln!(orange_bold, "Warning: Failed to remove autostart entry: {}", e);
    } else {
        showln!(
            gray_dim,
            "Removed ",
            yellow_bold,
            "autostart",
            gray_dim,
            " entry"
        );
    }
    
    Ok(())
}

fn create_uninstaller(install_dir: &std::path::Path) -> Result<()> {
    let uninstaller_path = install_dir.join("uninstall.bat");
    fs::write(&uninstaller_path, UNINSTALLER_CODE).context("Failed to create uninstaller script")?;
    
    showln!(
        gray_dim,
        "Created uninstaller script at ",
        yellow_bold,
 
        uninstaller_path.to_string_lossy()
    );
    Ok(())
}

fn update_environment_message() {
    unsafe {
        SendMessageTimeoutA(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            "Environment\0".as_ptr() as winapi::shared::minwindef::LPARAM,
            winapi::um::winuser::SMTO_ABORTIFHUNG,
            5000,
            ptr::null_mut(),
        );
    }
}

 