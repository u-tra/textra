use crate::parser::*;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::{mem, ptr};
use winapi::{
    shared::minwindef::{DWORD, FALSE, LPARAM, LPVOID, WPARAM},
    um::{
        fileapi::{CreateFileW, OPEN_EXISTING},
        handleapi::INVALID_HANDLE_VALUE,
        minwinbase::OVERLAPPED,
        synchapi::WaitForSingleObject,
        winbase::{
            ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, INFINITE,
            WAIT_OBJECT_0,
        },
        winnt::{
            FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        },
    },
};
use std::os::windows::ffi::OsStrExt;
use super::*;

const CONFIG_FILE_NAME: &str = "config.textra";

pub fn load_config() -> Result<TextraConfig, ParseError> {
    let config_path = get_config_path().unwrap();
    let config_str = fs::read_to_string(&config_path)
        .expect(&format!("Failed to read config file: {:?}", config_path));
    parse_textra_config(&config_str)
}

pub fn handle_edit_config() -> Result<(), io::Error> {
    let config_path = get_config_path().unwrap();
    if let Ok(code_path) = which::which("code") {
        std::process::Command::new(code_path)
            .arg(&config_path)
            .spawn()?;
    } else if let Ok(notepad_path) = which::which("notepad") {
        std::process::Command::new(notepad_path)
            .arg(&config_path)
            .spawn()?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No editor found. Please install Notepad or VS Code.",
        ));
    }
    Ok(())
}

pub fn display_config() {
    minimo::showln!(yellow_bold, "│ ", whitebg, " CONFIGURATION ");
    minimo::showln!(yellow_bold, "│ ");
    match load_config() {
        Ok(config) => {
            let config_path = get_config_path().unwrap();
            minimo::showln!(
                yellow_bold,
                "│ ",
                cyan_bold,
                "┌─ ",
                white_bold,
                config_path.display()
            );
            minimo::showln!(yellow_bold, "│ ", cyan_bold, "⇣ ");
            if !config.rules.is_empty() {
                for rule in &config.rules {
                    let (trigger, replace) = match &rule.replacement {
                        Replacement::Simple(text) => (&rule.triggers[0], text),
                        Replacement::Multiline(text) => (&rule.triggers[0], text),
                        Replacement::Code { language: _, content } => (&rule.triggers[0], content),
                    };
                    let trimmed = minimo::text::chop(replace, 50 - trigger.len())[0].clone();

                    minimo::showln!(
                        yellow_bold,
                        "│ ",
                        cyan_bold,
                        "▫ ",
                        gray_dim,
                        trigger,
                        cyan_bold,
                        " ⋯→ ",
                        white_bold,
                        trimmed
                    );
                }
            }
        }
        Err(e) => {
            minimo::showln!(red_bold, e);
        }
    }
    minimo::showln!(yellow_bold, "│ ");
    minimo::showln!(
        yellow_bold,
        "└───────────────────────────────────────────────────────────────"
    );
    minimo::showln!(gray_dim, "");
}

pub fn get_config_path() -> Result<PathBuf, io::Error> {
    let home_dir = dirs::document_dir().unwrap();
    let home_config_dir = home_dir.join("textra");
    let home_config_file = home_config_dir.join(CONFIG_FILE_NAME);

    if home_config_file.exists() {
        return Ok(home_config_file);
    }

    fs::create_dir_all(&home_config_dir)?;
    let home_config_file = home_config_dir.join(CONFIG_FILE_NAME);
    create_default_config(&home_config_file)?;
    Ok(home_config_file)
}

pub fn create_default_config(path: &Path) -> Result<(), io::Error> {
    fs::write(path, DEFAULT_CONFIG).expect("Failed to write default config file");
    Ok(())
}

pub fn watch_config(sender: std::sync::mpsc::Sender<Message>) -> Result<(), io::Error> {
    let config_path = get_config_path()?;
    let config_dir = config_path.parent().unwrap();

    unsafe {
        let dir_handle = CreateFileW(
            config_dir
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
                .as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            ptr::null_mut(),
        );

        if dir_handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().into());
        }

        let mut buffer = [0u8; 1024];
        let mut bytes_returned: DWORD = 0;
        let mut overlapped: OVERLAPPED = mem::zeroed();

        loop {
            let result = ReadDirectoryChangesW(
                dir_handle,
                buffer.as_mut_ptr() as LPVOID,
                buffer.len() as DWORD,
                FALSE,
                FILE_NOTIFY_CHANGE_LAST_WRITE,
                &mut bytes_returned,
                &mut overlapped,
                None,
            );

            if result == 0 {
                return Err(io::Error::last_os_error().into());
            }

            let event = WaitForSingleObject(dir_handle, INFINITE);
            if event != WAIT_OBJECT_0 {
                return Err(io::Error::last_os_error().into());
            }

            sender.send(Message::ConfigReload).unwrap();
        }
    }
}

// Remove GLOBAL_SENDER and set_global_sender as they're no longer needed
// with our new implementation

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