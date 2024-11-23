use anyhow::{Context, Result};
use io::Write;
use minimo::showln;
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


 
use serde::Deserialize;
 
use std::time::Duration;

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

pub   fn check_for_updates() -> Result<bool> {
    showln!(gray_dim, "checking for updates...");
    
    let current_version = get_current_version()?;
    let latest_release = get_latest_release()?;
    
    let latest_version = parse_version_from_tag(&latest_release.tag_name)?;
    
    Ok(latest_version > current_version)
}

pub fn handle_update() -> Result<()> {
    showln!(gray_dim, "starting update process...");

    let latest_release = get_latest_release()?;
    
    // Find textra.exe asset
    let textra_asset = latest_release.assets
        .iter()
        .find(|asset| asset.name == "textra.exe")
        .context("Could not find textra.exe in release assets")?;

    // Download new version to temporary location
    let temp_path = get_install_dir()?.join("textra.exe.new");
    download_file(&textra_asset.browser_download_url, &temp_path)?;

    // Stop running instance
    if is_service_running() {
        showln!(orange_bold, "stopping current instance for update...");
        handle_stop()?;
    }

    // Replace old executable
    let install_path = get_install_dir()?.join("textra.exe");
    fs::rename(&temp_path, &install_path)
        .context("Failed to replace old executable")?;

    // Restart service
    handle_run()?;

    showln!(green_bold, "update completed successfully!");
    Ok(())
}

fn get_latest_release() -> Result<GitHubRelease> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let response = client
        .get("https://api.github.com/repos/u-tra/textra/releases/latest")
        .header("User-Agent", "Textra-Updater")
        .send();
    if let Ok(response) = response {
        let release = response.json::<GitHubRelease>()?;
        Ok(release)
    } else {
        Err(anyhow::anyhow!("Failed to get latest release"))
    }
}

 fn download_file(url: &str, path: &PathBuf) -> Result<()> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "Textra-Updater")
        .send();
    

    if let Ok(response) = response {
      if response.status().is_success() {
        let mut file = File::create(path)?;
        let mut content = response.bytes()?;
        file.write_all(&content)?;
      }
    }
    Ok(())
}


#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u32, u32, u32, u32);

impl Version {
    fn parse(version_str: &str) -> Result<Self> {
        let parts: Vec<&str> = version_str.split('.').collect();
        Ok(Version(parts[0].parse().unwrap(), parts[1].parse().unwrap(), parts[2].parse().unwrap(), parts[3].parse().unwrap()))
    }
}

fn get_current_version() -> Result<Version> {
    // Parse current version from Cargo.toml or embedded version
    let version_str = env!("CARGO_PKG_VERSION");
    let current_version = Version::parse(version_str)
        .context("Failed to parse current version")?;
    Ok(current_version)
}

fn parse_version_from_tag(tag: &str) -> Result<Version> {
    // Convert v2024.11.23.002010 format to semver format
    let parts: Vec<&str> = tag.trim_start_matches('v').split('.').collect();
    if parts.len() != 4 {
        return Err(anyhow::anyhow!("Invalid version tag format"));
    }

    let version = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
    Version::parse(&version).context("Failed to parse version from tag")
}

pub   fn update_if_available() -> Result<()> {
    if check_for_updates()? {
        showln!(gray_dim, "new version available, updating...");
        handle_update()?;
    } else {
        showln!(gray_dim, "textra is up to date!");
    }
    Ok(())
}