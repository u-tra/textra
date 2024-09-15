use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use winapi::{shared::minwindef::{FALSE, LPVOID}, um::{fileapi::{CreateFileW, OPEN_EXISTING}, minwinbase::OVERLAPPED, winnt::{FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE}}};

use super::*;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum Message {
    KeyEvent(DWORD, WPARAM, LPARAM),
    ConfigReload,
    Quit,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Config {
    pub matches: Vec<Match>,
    #[serde(default)]
    pub backend: BackendConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Match {
    pub trigger: String,
    pub replacement: Replacement,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type")]
pub enum Replacement {
    Static {
        text: String,
        #[serde(default)]
        propagate_case: bool,
    },
    Dynamic {
        action: String,
    },
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct BackendConfig {
    #[serde(default = "default_key_delay")]
    pub key_delay: u64,
}

pub fn default_key_delay() -> u64 {
    10
}

pub fn load_config() -> Result<Config> {
    let config_path = get_config_path()?;
    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
    let config: Config = serde_yaml::from_str(&config_str)
        .with_context(|| format!("Failed to parse config file: {:?}", config_path))?;

    minimo::showln!(
        yellow_bold,
        "┌─",
        white_bold,
        " TEXTRA",
        yellow_bold,
        " ───────────────────────────────────────────────────────"
    );
    minimo::showln!(yellow_bold, "│ ", green_bold, config_path.display());
    if !config.matches.is_empty() {
        for match_rule in &config.matches {
            let (trigger, replace) = match &match_rule.replacement {
                Replacement::Static { text, .. } => (&match_rule.trigger, text),
                Replacement::Dynamic { action } => (&match_rule.trigger, action),
            };
            let trimmed = minimo::text::chop(replace, 50 - trigger.len())[0].clone();

            minimo::showln!(
                yellow_bold,
                "│ ",
                yellow_bold,
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
    minimo::showln!(
        yellow_bold,
        "└───────────────────────────────────────────────────────────────"
    );
    minimo::showln!(gray_dim, "");
    Ok(config)
}

pub fn get_config_path() -> Result<PathBuf> {
    // let current_dir = env::current_dir()?;
    // let current_dir_config = current_dir.join("config.yaml");
    // if current_dir_config.exists() {
    //     return Ok(current_dir_config);
    // }

    // if current_dir.file_name().unwrap() == "textra" {
    //     let config_file = current_dir.join("config.yaml");
    //     create_default_config(&config_file)?;
    //     return Ok(config_file);
    // }

    let home_dir = dirs::document_dir().unwrap();
    let home_config_dir = home_dir.join("textra");
    let home_config_file = home_config_dir.join("config.yaml");

    if home_config_file.exists() {
        return Ok(home_config_file);
    }

    fs::create_dir_all(&home_config_dir).context("Failed to create config directory")?;
    let home_config_file = home_config_dir.join("config.yaml");
    create_default_config(&home_config_file)?;
    Ok(home_config_file)
}

pub fn create_default_config(path: &Path) -> Result<()> {
    let default_config = Config {
        matches: vec![
            Match {
                trigger: "btw".to_string(),
                replacement: Replacement::Static {
                    text: "by the way".to_string(),
                    propagate_case: false,
                },
            },
            Match {
                trigger: ":date".to_string(),
                replacement: Replacement::Dynamic {
                    action: "{{date}}".to_string(),
                },
            },
            Match {
                trigger: ":time".to_string(),
                replacement: Replacement::Dynamic {
                    action: "{{time}}".to_string(),
                },
            },
            //common email responses
            Match {
                trigger: "pfa".to_string(),
                replacement: Replacement::Static {
                    text: "please find the attached information as requested".to_string(),
                    propagate_case: false,
                },
            },
            Match {
                trigger: "pftb".to_string(),
                replacement: Replacement::Static {
                    text: "please find the below information as required".to_string(),
                    propagate_case: false,
                },
            },
            Match {
                trigger: ":tst".to_string(),
                replacement: Replacement::Static {
                    text: "twinkle twinkle little star, how i wonder what you are,\nup above the world so high,\nlike a diamond in the sky".to_string(),
                    propagate_case: false,
                },
            },
        ],
        backend: BackendConfig { key_delay: 10 },
    };
    let yaml = serde_yaml::to_string(&default_config)?;
    fs::write(path, yaml).context("Failed to write default config file")?;
    Ok(())
}

pub fn watch_config(sender: Sender<Message>) -> Result<()> {
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

pub static GLOBAL_SENDER: Lazy<Mutex<Option<Sender<Message>>>> = Lazy::new(|| Mutex::new(None));

pub fn set_global_sender(sender: Sender<Message>) {
    let mut global_sender = GLOBAL_SENDER.lock();
    *global_sender = Some(sender);
}
