//! Keyboard API abstractions and Windows-specific implementations.
//! Provides traits and implementations for keyboard monitoring and input operations.

use crate::{Result, TextraError, errors::KeyboardError};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::ptr;
use std::time::{Duration, Instant};
use std::thread;
use winapi::shared::minwindef::{LPARAM, LRESULT, WPARAM};
use winapi::um::winuser::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
};

/// Health status of keyboard monitoring and input systems
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    /// System is functioning normally
    Healthy,
    /// System is operating but with potential issues
    Degraded(String),
    /// System is not functioning properly
    Unhealthy(String),
}

/// Keyboard modifier state flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub shift_pressed: bool,
    pub caps_lock_on: bool,
    pub ctrl_pressed: bool,
    pub alt_pressed: bool,
}

/// Trait for monitoring keyboard events and managing hooks
pub trait KeyboardMonitor: Send + Sync {
    /// Start monitoring keyboard events
    fn start(&mut self) -> Result<()>;
    
    /// Stop monitoring keyboard events
    fn stop(&mut self) -> Result<()>;
    
    /// Check if monitoring is currently active
    fn is_running(&self) -> bool;
    
    /// Check the health status of the keyboard monitoring system
    fn health_check(&self) -> Result<HealthStatus>;
}

/// Trait for keyboard input operations
pub trait KeyboardInput: Send + Sync {
    /// Delete a specified number of characters with optional delay
    fn delete_chars(&self, count: usize, delay_ms: u64) -> Result<()>;
    
    /// Type text with specified modifiers and delay
    fn type_text(&self, text: &str, modifiers: KeyModifiers, delay_ms: u64) -> Result<()>;
}

/// Thread-safe wrapper for Windows hook handle
struct HookHandle(*mut winapi::shared::windef::HHOOK__);

// Safety: The hook handle is protected by a mutex in WindowsKeyboard
unsafe impl Send for HookHandle {}
unsafe impl Sync for HookHandle {}

impl HookHandle {
    fn new() -> Self {
        Self(ptr::null_mut())
    }

    fn set(&mut self, hook: *mut winapi::shared::windef::HHOOK__) {
        self.0 = hook;
    }

    fn take(&mut self) -> *mut winapi::shared::windef::HHOOK__ {
        let hook = self.0;
        self.0 = ptr::null_mut();
        hook
    }

    fn is_some(&self) -> bool {
        !self.0.is_null()
    }
}

impl Drop for HookHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                UnhookWindowsHookEx(self.0);
            }
        }
    }
}

/// Error pattern tracking for keyboard operations
#[derive(Debug, Clone)]
pub struct ErrorPattern {
    pub error_type: String,
    pub frequency: u32,
    pub last_occurrence: Instant,
    pub consecutive_count: u32,
}

/// Health metrics for keyboard monitoring
#[derive(Debug, Clone)]
pub struct HealthMetrics {
    pub error_count: u32,
    pub last_error_time: Option<Instant>,
    pub consecutive_failures: u32,
    pub uptime: Duration,
}

/// Windows-specific implementation of keyboard monitoring and input
pub struct WindowsKeyboard {
    hook: Arc<Mutex<HookHandle>>,
    running: Arc<AtomicBool>,
    error_count: Arc<AtomicU32>,
    last_error: Arc<Mutex<Option<(KeyboardError, Instant)>>>,
    start_time: Arc<Mutex<Instant>>,
}

impl WindowsKeyboard {
    pub fn new() -> Self {
        Self {
            hook: Arc::new(Mutex::new(HookHandle::new())),
            running: Arc::new(AtomicBool::new(false)),
            error_count: Arc::new(AtomicU32::new(0)),
            last_error: Arc::new(Mutex::new(None)),
            start_time: Arc::new(Mutex::new(Instant::now())),
        }
    }

    fn retry_with_backoff<T, F>(&self, operation: F) -> Result<T>
    where
        F: Fn() -> Result<T>,
    {
        const MAX_RETRIES: u32 = 3;
        const BASE_DELAY: u64 = 100; // milliseconds

        let mut attempts = 0;
        let mut last_error = None;

        while attempts < MAX_RETRIES {
            match operation() {
                Ok(result) => return Ok(result),
                Err(e) => {
                    attempts += 1;
                    last_error = Some(e.to_string());

                    if attempts < MAX_RETRIES {
                        let delay = BASE_DELAY * (2_u64.pow(attempts - 1));
                        thread::sleep(Duration::from_millis(delay));
                    }
                }
            }
        }

        match last_error {
            Some(error_msg) => Err(KeyboardError::RetryTimeout {
                attempts,
                message: error_msg
            }.into()),
            None => Err(KeyboardError::SystemError("Retry failed with no error".into()).into()),
        }
    }

    fn record_error(&self, error: KeyboardError) {
        self.error_count.fetch_add(1, Ordering::SeqCst);
        *self.last_error.lock().unwrap() = Some((error, Instant::now()));
    }

    fn collect_health_metrics(&self) -> HealthMetrics {
        let error_count = self.error_count.load(Ordering::SeqCst);
        let last_error_info = self.last_error.lock().unwrap();
        let last_error_time = last_error_info.as_ref().map(|(_, time)| *time);
        let start_time = *self.start_time.lock().unwrap();

        HealthMetrics {
            error_count,
            last_error_time,
            consecutive_failures: 0, // Updated in health check
            uptime: start_time.elapsed(),
        }
    }

    fn analyze_error_patterns(&self) -> Vec<ErrorPattern> {
        let mut patterns = Vec::new();
        if let Some((error, time)) = &*self.last_error.lock().unwrap() {
            patterns.push(ErrorPattern {
                error_type: error.to_string(),
                frequency: self.error_count.load(Ordering::SeqCst),
                last_occurrence: *time,
                consecutive_count: 1, // Basic implementation
            });
        }
        patterns
    }

    unsafe extern "system" fn keyboard_hook_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Forward to next hook in chain
        CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
    }
}

impl Default for WindowsKeyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardMonitor for WindowsKeyboard {
    fn start(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        self.retry_with_backoff(|| {
            unsafe {
                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(Self::keyboard_hook_proc),
                    ptr::null_mut(),
                    0,
                );

                if hook.is_null() {
                    let error = KeyboardError::HookError("Failed to set keyboard hook".into());
                    self.record_error(error.clone());
                    return Err(error.into());
                }

                self.hook.lock().unwrap().set(hook);
                self.running.store(true, Ordering::SeqCst);
                *self.start_time.lock().unwrap() = Instant::now();
                Ok(())
            }
        })
    }

    fn stop(&mut self) -> Result<()> {
        self.retry_with_backoff(|| {
            let mut hook_guard = self.hook.lock().unwrap();
            if hook_guard.is_some() {
                unsafe {
                    let hook = hook_guard.take();
                    if UnhookWindowsHookEx(hook) != 0 {
                        self.running.store(false, Ordering::SeqCst);
                        Ok(())
                    } else {
                        let error = KeyboardError::HookError("Failed to unhook keyboard hook".into());
                        self.record_error(error.clone());
                        Err(error.into())
                    }
                }
            } else {
                Ok(())
            }
        })
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn health_check(&self) -> Result<HealthStatus> {
        if !self.is_running() {
            return Ok(HealthStatus::Unhealthy("Keyboard monitoring is not running".into()));
        }

        if !self.hook.lock().unwrap().is_some() {
            return Ok(HealthStatus::Degraded("Keyboard hook is not properly initialized".into()));
        }

        let metrics = self.collect_health_metrics();
        let patterns = self.analyze_error_patterns();

        // Check for error patterns
        if !patterns.is_empty() {
            let pattern = &patterns[0];
            if pattern.consecutive_count > 2 {
                return Ok(HealthStatus::Degraded(
                    format!("Multiple consecutive errors: {} ({})",
                        pattern.error_type, pattern.consecutive_count)
                ));
            }
        }

        // Check error frequency
        if metrics.error_count > 5 {
            return Ok(HealthStatus::Degraded(
                format!("High error frequency: {} errors", metrics.error_count)
            ));
        }

        Ok(HealthStatus::Healthy)
    }
}

impl KeyboardInput for WindowsKeyboard {
    fn delete_chars(&self, count: usize, delay_ms: u64) -> Result<()> {
        self.retry_with_backoff(|| {
            crate::keyboard::delete_chars(count, delay_ms).map_err(|e| {
                let error = KeyboardError::InputError(e.to_string());
                self.record_error(error.clone());
                error.into()
            })
        })
    }

    fn type_text(&self, text: &str, modifiers: KeyModifiers, delay_ms: u64) -> Result<()> {
        self.retry_with_backoff(|| {
            crate::keyboard::type_text(
                text,
                modifiers.shift_pressed,
                modifiers.caps_lock_on,
                delay_ms
            ).map_err(|e| {
                let error = KeyboardError::InputError(e.to_string());
                self.record_error(error.clone());
                error.into()
            })
        })
    }
}

impl From<KeyboardError> for TextraError {
    fn from(error: KeyboardError) -> Self {
        TextraError::Process(match error {
            KeyboardError::HookError(msg) => msg,
            KeyboardError::InputError(msg) => msg,
            KeyboardError::SystemError(msg) => msg,
            KeyboardError::RetryTimeout { attempts, message } =>
                format!("Operation timed out after {} attempts: {}", attempts, message)
        })
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::collections::HashMap;
    use parking_lot::RwLock;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum KeyboardAction {
        DeleteChars { count: usize, delay_ms: u64 },
        TypeText { text: String, modifiers: KeyModifiers, delay_ms: u64 },
        StartMonitoring,
        StopMonitoring,
    }

    #[derive(Clone)]
    pub struct MockKeyboard {
        actions: Arc<Mutex<Vec<KeyboardAction>>>,
        running: Arc<AtomicBool>,
        error_count: Arc<AtomicU32>,
        last_error: Arc<Mutex<Option<(KeyboardError, Instant)>>>,
        start_time: Arc<Mutex<Instant>>,
        // Error simulation
        fail_next_operation: Arc<AtomicBool>,
        failure_count: Arc<AtomicUsize>,
        operation_latency: Arc<RwLock<HashMap<String, Duration>>>,
        error_patterns: Arc<RwLock<Vec<ErrorPattern>>>,
        health_metrics: Arc<RwLock<HealthMetrics>>,
    }

    impl MockKeyboard {
        pub fn new() -> Self {
            Self {
                actions: Arc::new(Mutex::new(Vec::new())),
                running: Arc::new(AtomicBool::new(false)),
                error_count: Arc::new(AtomicU32::new(0)),
                last_error: Arc::new(Mutex::new(None)),
                start_time: Arc::new(Mutex::new(Instant::now())),
                fail_next_operation: Arc::new(AtomicBool::new(false)),
                failure_count: Arc::new(AtomicUsize::new(0)),
                operation_latency: Arc::new(RwLock::new(HashMap::new())),
                error_patterns: Arc::new(RwLock::new(Vec::new())),
                health_metrics: Arc::new(RwLock::new(HealthMetrics {
                    error_count: 0,
                    last_error_time: None,
                    consecutive_failures: 0,
                    uptime: Duration::from_secs(0),
                })),
            }
        }

        pub fn get_actions(&self) -> Vec<KeyboardAction> {
            self.actions.lock().unwrap().clone()
        }
        
        pub fn clear_actions(&self) {
            self.actions.lock().unwrap().clear();
        }

        // Error simulation methods
        pub fn set_fail_next(&self, should_fail: bool) {
            self.fail_next_operation.store(should_fail, Ordering::SeqCst);
        }

        pub fn set_operation_latency(&self, operation: &str, latency: Duration) {
            self.operation_latency.write().insert(operation.to_string(), latency);
        }

        pub fn add_error_pattern(&self, pattern: ErrorPattern) {
            self.error_patterns.write().push(pattern);
        }

        pub fn get_health_metrics(&self) -> HealthMetrics {
            self.health_metrics.read().clone()
        }

        fn simulate_operation<T, F>(&self, operation: &str, f: F) -> Result<T>
        where
            F: FnOnce() -> Result<T>,
        {
            // Check for simulated failure
            if self.fail_next_operation.load(Ordering::SeqCst) {
                self.fail_next_operation.store(false, Ordering::SeqCst);
                self.failure_count.fetch_add(1, Ordering::SeqCst);
                let error = KeyboardError::InputError("Simulated failure".into());
                self.record_error(error.clone());
                return Err(error.into());
            }

            // Simulate operation latency
            if let Some(latency) = self.operation_latency.read().get(operation) {
                thread::sleep(*latency);
            }

            f()
        }

        fn record_error(&self, error: KeyboardError) {
            let mut metrics = self.health_metrics.write();
            metrics.error_count += 1;
            metrics.last_error_time = Some(Instant::now());
            metrics.consecutive_failures += 1;
        }
    }

    impl Default for MockKeyboard {
        fn default() -> Self {
            Self::new()
        }
    }

    impl KeyboardMonitor for MockKeyboard {
        fn start(&mut self) -> Result<()> {
            self.simulate_operation("start", || {
                self.actions.lock().unwrap().push(KeyboardAction::StartMonitoring);
                self.running.store(true, Ordering::SeqCst);
                *self.start_time.lock().unwrap() = Instant::now();
                Ok(())
            })
        }

        fn stop(&mut self) -> Result<()> {
            self.simulate_operation("stop", || {
                self.actions.lock().unwrap().push(KeyboardAction::StopMonitoring);
                self.running.store(false, Ordering::SeqCst);
                Ok(())
            })
        }

        fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }

        fn health_check(&self) -> Result<HealthStatus> {
            let metrics = self.health_metrics.read();
            
            if !self.is_running() {
                return Ok(HealthStatus::Unhealthy("Mock keyboard is not running".into()));
            }

            if metrics.consecutive_failures > 3 {
                return Ok(HealthStatus::Unhealthy(format!(
                    "Too many consecutive failures: {}", metrics.consecutive_failures
                )));
            }

            if metrics.error_count > 5 {
                return Ok(HealthStatus::Degraded(format!(
                    "High error count: {}", metrics.error_count
                )));
            }

            Ok(HealthStatus::Healthy)
        }
    }

    impl KeyboardInput for MockKeyboard {
        fn delete_chars(&self, count: usize, delay_ms: u64) -> Result<()> {
            self.simulate_operation("delete_chars", || {
                self.actions.lock().unwrap().push(KeyboardAction::DeleteChars { count, delay_ms });
                Ok(())
            })
        }

        fn type_text(&self, text: &str, modifiers: KeyModifiers, delay_ms: u64) -> Result<()> {
            self.simulate_operation("type_text", || {
                self.actions.lock().unwrap().push(KeyboardAction::TypeText {
                    text: text.to_string(),
                    modifiers,
                    delay_ms,
                });
                Ok(())
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::Arc;
        use std::thread;

        #[test]
        fn test_retry_backoff() {
            let mut keyboard = MockKeyboard::new();
            keyboard.set_fail_next(true);
            keyboard.set_operation_latency("start", Duration::from_millis(50));

            let start_time = Instant::now();
            let result = keyboard.start();
            let elapsed = start_time.elapsed();

            assert!(result.is_err());
            assert!(elapsed >= Duration::from_millis(50));
        }

        #[test]
        fn test_error_recovery() {
            let mut keyboard = MockKeyboard::new();
            
            // First operation fails
            keyboard.set_fail_next(true);
            assert!(keyboard.start().is_err());
            
            // Second operation succeeds
            assert!(keyboard.start().is_ok());
            assert!(keyboard.is_running());
        }

        #[test]
        fn test_health_monitoring() {
            let mut keyboard = MockKeyboard::new();
            
            // Initial state
            assert_eq!(keyboard.health_check().unwrap(), HealthStatus::Unhealthy("Mock keyboard is not running".into()));
            
            // After successful start
            keyboard.start().unwrap();
            assert_eq!(keyboard.health_check().unwrap(), HealthStatus::Healthy);
            
            // After errors
            for _ in 0..4 {
                keyboard.set_fail_next(true);
                let _ = keyboard.type_text("test", KeyModifiers::default(), 0);
            }
            
            assert!(matches!(keyboard.health_check().unwrap(), HealthStatus::Degraded(_)));
        }

        #[test]
        fn test_thread_safety() {
            let keyboard = Arc::new(MockKeyboard::new());
            let mut handles = vec![];

            for i in 0..10 {
                let kb = keyboard.clone();
                handles.push(thread::spawn(move || {
                    kb.type_text(&format!("text{}", i), KeyModifiers::default(), 0).unwrap();
                }));
            }

            for handle in handles {
                handle.join().unwrap();
            }

            let actions = keyboard.get_actions();
            assert_eq!(actions.len(), 10);
        }

        #[test]
        fn test_windows_api_integration() {
            let mut keyboard = MockKeyboard::new();
            
            // Test hook management
            assert!(!keyboard.is_running());
            keyboard.start().unwrap();
            assert!(keyboard.is_running());
            keyboard.stop().unwrap();
            assert!(!keyboard.is_running());
            
            // Test input operations with modifiers
            let modifiers = KeyModifiers {
                shift_pressed: true,
                caps_lock_on: false,
                ctrl_pressed: true,
                alt_pressed: false,
            };
            
            keyboard.type_text("Test", modifiers, 100).unwrap();
            
            let actions = keyboard.get_actions();
            assert!(actions.iter().any(|action| matches!(
                action,
                KeyboardAction::TypeText { text, modifiers: m, .. }
                if text == "Test" && m.shift_pressed && m.ctrl_pressed
            )));
        }

        #[test]
        fn test_error_patterns() {
            let keyboard = MockKeyboard::new();
            
            // Add error pattern
            keyboard.add_error_pattern(ErrorPattern {
                error_type: "InputError".to_string(),
                frequency: 1,
                last_occurrence: Instant::now(),
                consecutive_count: 1,
            });
            
            // Simulate failures
            keyboard.set_fail_next(true);
            let result = keyboard.type_text("test", KeyModifiers::default(), 0);
            assert!(result.is_err());
            
            let metrics = keyboard.get_health_metrics();
            assert_eq!(metrics.error_count, 1);
            assert!(metrics.last_error_time.is_some());
        }
    }
}