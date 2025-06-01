// src/errors.rs
use std::path::PathBuf;
use anyhow;
use thiserror::Error;
use crate::Rule; // This assumes Rule from lib.rs (via TextraParser) is accessible.

#[derive(Debug, Error, Clone)]
pub enum KeyboardError {
    #[error("Failed to set keyboard hook: {0}")]
    HookError(String),
    
    #[error("Input operation failed: {0}")]
    InputError(String),
    
    #[error("System call failed: {0}")]
    SystemError(String),
    
    #[error("Operation timed out after {attempts} attempts: {message}")]
    RetryTimeout {
        attempts: u32,
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to load configuration: {0}")]
    Load(String), // Generic load error
    #[error("Failed to parse configuration: {0}")]
    Parse(#[from] pest::error::Error<Rule>), // Direct from pest error
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("I/O error during config operation: {source}")]
    Io { #[from] source: std::io::Error },
    #[error("Could not find home directory for configuration")]
    HomeDirectoryNotFound,
    #[error("Failed to create configuration directory: {source}")]
    CreateConfigDir { source: std::io::Error },
    #[error("Failed to write default configuration: {source}")]
    WriteDefaultConfig { source: std::io::Error },
    #[error("Failed to read config file at {path}: {source}")]
    ReadConfig { path: PathBuf, source: std::io::Error },
}

#[derive(Debug, Error)]
pub enum TextraError {
    #[error(transparent)] // Use transparent for better error messages from ConfigError
    Config(#[from] ConfigError),
    #[error("IPC communication failed: {0}")]
    Ipc(String),
    #[error("Keyboard hook failed: {source}")]
    KeyboardHook { source: std::io::Error }, // Specific to keyboard hook
    #[error("I/O error: {source}")]
    Io { #[from] source: std::io::Error }, // General I/O, distinct from ConfigError::Io
    #[error("Serde JSON error: {source}")]
    SerdeJson { #[from] source: serde_json::Error },
    #[error("Process management error: {0}")]
    Process(String),
    #[error("Failed to get current executable path: {source}")]
    CurrentExePath { source: std::io::Error },
    #[error("Failed to get executable directory")]
    ExeDirectory,
    #[error("Failed to start process '{name}': {source}")]
    StartProcess { name: String, source: std::io::Error },
    #[error("Failed to stop process '{name}': {source}")]
    StopProcess { name: String, source: std::io::Error },
    #[error("Failed to parse version from tag '{tag}': {reason}")]
    VersionParse { tag: String, reason: String },
    #[error("HTTP client error: {source}")]
    HttpClient { #[from] source: reqwest::Error },
    #[error("GitHub API error: {0}")]
    GitHubApi(String),
    #[error("Tempfile error: {source}")]
    TempFile { source: std::io::Error },
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

// Global Result type alias
pub type Result<T> = std::result::Result<T, TextraError>;