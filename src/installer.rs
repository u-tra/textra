use anyhow::{Context, Result};
use io::Write;
use minimo::showln;
use serde::Deserialize;
use std::env;
use std::fs;
use std::fs::File;
use std::path::PathBuf;
use std::ptr;
use winapi::um::winuser::{SendMessageTimeoutA, HWND_BROADCAST, WM_SETTINGCHANGE};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use super::*;

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

pub fn auto_install() -> Result<()> {
    if !is_installed() {
     
        handle_install().context("Failed to install textra")?;
    }

    if !is_service_running() {
        handle_run().context("Failed to start daemon")?;
    };
    Ok(())
}


pub fn is_installed() -> bool {
    let install_dir = get_install_dir().unwrap();
    install_dir.join("textra.exe").exists()
}

pub fn handle_install() -> Result<()> {
    showln!(gray_dim, "trying to install textra...");

    if is_service_running() {
        showln!(
            orange_bold,
            "an instance of textra is already running, stopping it..."
        );  
        handle_stop().context("Failed to stop running instance")?;
    }

    let exe_path = env::current_exe().context("Failed to get current executable path")?;
    let install_dir = get_install_dir()?;

    let install_path = install_dir.join("textra.exe");
    showln!(gray_dim, "copying ", yellow_bold, "textra.exe", gray_dim, " to ", yellow_bold, install_dir.to_string_lossy());
    fs::copy(&exe_path, &install_path).context("Failed to copy executable to install directory")?;

    add_to_path(&install_dir).context("Failed to add Textra to PATH")?;
    set_autostart(&install_path).context("Failed to set autostart")?;
    create_uninstaller(&install_dir).context("Failed to create uninstaller")?;
    handle_run().context("Failed to start service")?;
 
    Ok(())
}

pub fn is_running_from_install_dir() -> bool {
    if let Ok(exe_path) = env::current_exe() {
        if let Some(home_dir) = dirs::home_dir() {
            if exe_path.starts_with(&home_dir.join(".textra")) {
                return true;
            }
        }
    }
    false
}

pub fn handle_uninstall() -> Result<()> {
    showln!(gray_dim, "uninstalling textra from your system...");
   
    match handle_stop().context("Failed to stop running instance") {
        Ok(_) => {
            showln!(gray_dim, "textra service ", red_bold, "stopped.");
        }
        Err(e) => {
            showln!(orange_bold, "oops! couldn't stop textra service. you can stop it manually by running uninstall.bat in .textra folder");
        }
    }
    match remove_autostart().context("Failed to remove autostart entry") {
        Ok(_) => {
            showln!(gray_dim, "autostart entry removed.");
        }
        Err(e) => {
            showln!(gray_dim, "huh! couldn't remove autostart entry. maybe it's already removed.");
        }
    }

    match remove_from_path().context("Failed to remove textra from path") {
        Ok(_) => {
            showln!(gray_dim, "textra removed from path.");
        }
        Err(e) => {
            showln!(gray_dim, "couldn't find textra in path. skipping...");
        }
    }

    let install_dir = get_install_dir()?;
    match fs::remove_dir_all(&install_dir).context("Failed to remove installation directory") {
        Ok(_) => {
            showln!(gray_dim, "textra removed from path.");
        }
        Err(e) => {
            showln!(gray_dim, "couldn't find textra in path. skipping...");
        }
    }

    showln!(gray_dim, "textra have been ", red_bold, "uninstalled", gray_dim, " from your system.");
    Ok(())
}

fn get_install_dir() -> Result<PathBuf> {
    let d = dirs::home_dir()
        .map(|dir| dir.join(".textra"))
        .context("Failed to determine local data directory")?;
    fs::create_dir_all(&d).context("Failed to create installation directory")?;
    Ok(d)
}

fn add_to_path(install_dir: &std::path::Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu
        .create_subkey("Environment")
        .context("Failed to open Environment registry key")?;

    let current_path: String = env
        .get_value("PATH")
        .context("Failed to get current PATH")?;
    if !current_path.contains(&install_dir.to_string_lossy().to_string()) {
        let new_path = format!("{};{}", current_path, install_dir.to_string_lossy());
        env.set_value("PATH", &new_path)
            .context("Failed to set new PATH")?;
        showln!(
            gray_dim,
            "added to ",
            green_bold,
            "path ",
            gray_dim,
            "environment variable."
        );
        showln!(
            gray_dim,
            "now you can access textra by typing",
            yellow_bold,
            " textra ",
            gray_dim,
            "in your terminal."
        );
    } else {
        //dont do anything
    };

    update_environment_message();
 
    Ok(())
}

fn set_autostart(install_path: &std::path::Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(AUTO_START_PATH)
        .context("Failed to open Run registry key")?;
    let command = format!(
        r#"cmd /C start /min "" "{}" run"#,
        install_path.to_string_lossy()
    );
    key.set_value("Textra", &command)
        .context("Failed to set autostart registry value")?;

    showln!(
        gray_dim,
        "activated",
        green_bold,
        " automatic startup."
    );
    Ok(())
}

pub fn check_autostart() -> bool {
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
    let (env, _) = hkcu
        .create_subkey("Environment")
        .context("Failed to open Environment registry key")?;

    let current_path: String = env
        .get_value("PATH")
        .context("Failed to get current PATH")?;
    let install_dir = get_install_dir()?;
    let new_path: Vec<&str> = current_path
        .split(';')
        .filter(|&p| p != install_dir.to_str().unwrap())
        .collect();
    let new_path = new_path.join(";");

    env.set_value("PATH", &new_path)
        .context("Failed to set new PATH")?;

    update_environment_message();

    showln!(
        gray_dim,
        "removed textra from path environment variable."
    );
    Ok(())
}

fn remove_autostart() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(AUTO_START_PATH)
        .context("Failed to open Run registry key")?;

    if let Err(e) = key.delete_value("Textra") {
        showln!(
            orange_bold,
            "Warning: Failed to remove autostart entry: {}",
            e
        );
    } else {
        showln!(
            gray_dim,
            "cancelling autostart..."
        );
    }

    Ok(())
}

fn create_uninstaller(install_dir: &std::path::Path) -> Result<()> {
    let uninstaller_path = install_dir.join("uninstall.bat");
    fs::write(&uninstaller_path, UNINSTALLER_CODE)
        .context("Failed to create uninstaller script")?;

    showln!(
        gray_dim,
        "textra have been ",
        green_bold, 
        "successfully installed ",
        gray_dim,
        "on this system."
    );

    showln!(
        gray_dim,
        "you can uninstall textra by running ",
        yellow_bold,
        "textra uninstall",
        gray_dim,
        " in the terminal"
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

 
use std::process::Command;
use std::time::Duration;
 

#[derive(Deserialize, Debug)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize, Debug)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    parts: Vec<u32>,
}

impl Version {
    fn parse(version_str: &str) -> Result<Self> {
        // Remove 'v' prefix if present
        let version_str = version_str.trim_start_matches('v');
        
        // Split and parse all parts as numbers
        let parts: Result<Vec<u32>, _> = version_str
            .split('.')
            .map(|s| s.parse::<u32>())
            .collect();

        let parts = parts.context(format!("Invalid version format: {}", version_str))?;
        Ok(Version { parts })
    }

    fn to_string(&self) -> String {
        self.parts.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(".")
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare each part, padding shorter version with 0s
        let max_len = self.parts.len().max(other.parts.len());
        for i in 0..max_len {
            let self_part = self.parts.get(i).copied().unwrap_or(0);
            let other_part = other.parts.get(i).copied().unwrap_or(0);
            
            match self_part.cmp(&other_part) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    }
}


const CREATE_NO_WINDOW: u32 = 0x08000000;
const DETACHED_PROCESS: u32 = 0x00000008;
pub fn handle_update() -> Result<()> {
    let latest_release = get_latest_release()?;
    let latest_version = parse_version_from_tag(&latest_release.tag_name)?;
    println!("assets: {:?}", latest_release.assets);
    let textra_asset = latest_release.assets
        .iter()
        .find(|asset| asset.name == "textra.exe")
        .context("Could not find textra.exe in release assets")?;

    // Get paths
    let install_dir = get_install_dir()?;
    let current_exe = env::current_exe()?;
    let new_exe_path = install_dir.join("textra.new.exe");
    let update_script_path = install_dir.join("update.bat");

    // Download new version first
    showln!(gray_dim, "downloading version ", yellow_bold, &latest_version.to_string());
    download_file(&textra_asset.browser_download_url, &new_exe_path)?;

    // Create update batch script
    let batch_script = format!(
        r#"@echo off
setlocal enabledelayedexpansion

rem Wait a bit for parent process to exit
timeout /t 1 /nobreak >nul

rem Kill any running instances of textra
taskkill /F /IM textra.exe /T >nul 2>&1
timeout /t 1 /nobreak >nul

:RETRY_COPY
rem Try to copy new version over old version
copy /Y "{new}" "{current}" >nul 2>&1
if !errorlevel! neq 0 (
    timeout /t 1 /nobreak >nul
    goto RETRY_COPY
)

rem Start new version
start "" "{current}" run

rem Clean up
del "{new}" >nul 2>&1
del "%~f0"
"#,
        new = new_exe_path.display(),
        current = current_exe.display()
    );

    fs::write(&update_script_path, batch_script)?;

    // Launch the update script and exit
    showln!(gray_dim, "starting update process...");
   // Launch update script with explicit path and working directory
   let status = Command::new("cmd")
   .args(&["/C", update_script_path.to_str().unwrap()])
   .current_dir(&install_dir)
   .creation_flags(CREATE_NO_WINDOW)
   .spawn()
   .context("Failed to start update process")?;

 

    showln!(gray_dim, "update prepared, restarting textra...");
    std::process::exit(0);
}

fn download_file(url: &str, path: &PathBuf) -> Result<()> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "Textra-Updater")
        .send()
        .context("Failed to download update")?;

    if response.status().is_success() {
        let content = response.bytes()
            .context("Failed to read download content")?;
        let mut file = File::create(path)
            .context("Failed to create temporary file")?;
        file.write_all(&content)
            .context("Failed to write update to disk")?;
        Ok(())
    } else {
        Err(anyhow::anyhow!("Download failed with status: {}", response.status()))
    }
}

fn get_latest_release() -> Result<GitHubRelease> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let response = client
        .get("https://api.github.com/repos/u-tra/textra/releases/latest")
        .header("User-Agent", "Textra-Updater")
        .send()
        .context("Failed to contact GitHub API")?;

    if response.status().is_success() {
        response.json::<GitHubRelease>()
            .context("Failed to parse GitHub response")
    } else {
        Err(anyhow::anyhow!("GitHub API returned status: {}", response.status()))
    }
}

 

fn get_current_version() -> Result<Version> {
    let version_str = env!("CARGO_PKG_VERSION");
    Version::parse(version_str)
        .context("Failed to parse current version")
}

fn parse_version_from_tag(tag: &str) -> Result<Version> {
    Version::parse(tag)
        .context(format!("Failed to parse version from tag: {}", tag))
}

pub fn update_if_available() -> Result<()> {
    let current_version = get_current_version()?;
    showln!(gray_dim, "checking for updates (current version: ", yellow_bold, &current_version.to_string(), gray_dim, ")");

    match check_for_updates() {
        Ok(true) => {
            showln!(gray_dim, "new version available, preparing update...");
            handle_update()
        }
        Ok(false) => {
            showln!(gray_dim, "textra is up to date!");
            Ok(())
        }
        Err(e) => {
            showln!(orange_bold, "failed to check for updates: {}", e);
            Err(e)
        }
    }
}

pub fn check_for_updates() -> Result<bool> {
    let current_version = get_current_version()?;
    showln!(gray_dim, "current version: ", yellow_bold, &current_version.to_string());
    
    match get_latest_release() {
        Ok(latest_release) => {
            let latest_version = parse_version_from_tag(&latest_release.tag_name)?;
            showln!(gray_dim, "latest version: ", yellow_bold, &latest_version.to_string());
            
            Ok(latest_version > current_version)
        }
        Err(e) => {
            showln!(orange_bold, "failed to check for updates: {}", e);
            Ok(false)
        }
    }
}