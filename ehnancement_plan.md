# Textra Development Plan
## Comprehensive Implementation Roadmap

---

## 📊 Current State Analysis

### ✅ **Existing Strengths**
- **Microservices Architecture**: Clean separation of CLI, Core Daemon, and Overlay
- **Configuration System**: Custom Pest parser for `.textra` files
- **IPC Communication**: Named pipes for inter-service communication
- **Basic Functionality**: Text expansion and template overlay working
- **Installation System**: Auto-start, PATH management, update mechanism

### ⚠️ **Critical Issues to Address**
- **Complex Windows API Integration**: Direct keyboard hooks are fragile
- **Performance Bottlenecks**: Real-time processing without optimization
- **Error Handling**: Excessive `unwrap()` calls and basic error management
- **Testing Infrastructure**: No test coverage or CI/CD
- **IPC Overengineering**: Named pipes might be overkill for simple communication
- **UI Dependencies**: Heavy web-view for simple overlay functionality

---

## 🎯 Development Phases

### **Phase 1: Foundation Stabilization** *(Weeks 1-3)*

#### 1.1 Error Handling & Logging Overhaul
```rust
// Priority: CRITICAL
// Goal: Replace all unwrap() calls with proper error handling
```

**Tasks:**
- [ ] **Audit all `unwrap()` calls** across codebase
- [ ] **Implement comprehensive error types**
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum TextraError {
      #[error("Configuration error: {0}")]
      Config(#[from] ConfigError),
      #[error("IPC communication failed: {0}")]
      Ipc(String),
      #[error("Keyboard hook failed: {source}")]
      Keyboard { source: io::Error },
      // ... more specific error types
  }
  ```
- [ ] **Enhanced logging system**
  - Structured logging with `tracing` instead of `log`
  - Log rotation and configurable levels
  - Performance metrics logging
- [ ] **Graceful degradation mechanisms**
  - Fallback modes when components fail
  - Auto-recovery for transient failures

#### 1.2 Testing Infrastructure
```rust
// Priority: HIGH
// Goal: Establish comprehensive testing framework
```

**Tasks:**
- [ ] **Unit test foundation**
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      
      #[test]
      fn test_trigger_detection() {
          // Test keyboard trigger detection
      }
      
      #[test] 
      fn test_text_replacement() {
          // Test replacement logic
      }
  }
  ```
- [ ] **Integration tests for IPC**
- [ ] **Mock Windows APIs for testing**
  ```rust
  #[cfg(test)]
  pub trait KeyboardApi {
      fn send_input(&self, input: &[INPUT]) -> Result<u32>;
      fn set_hook(&self, proc: HOOKPROC) -> Result<HHOOK>;
  }
  ```
- [ ] **CI/CD Pipeline Setup**
  - GitHub Actions for automated testing
  - Cross-platform compatibility checks
  - Performance regression testing

#### 1.3 Performance Optimization
```rust
// Priority: HIGH  
// Goal: Optimize real-time keyboard processing
```

**Tasks:**
- [ ] **Keyboard processing optimization**
  ```rust
  // Use lock-free data structures for hot path
  use crossbeam::queue::ArrayQueue;
  
  static KEY_BUFFER: ArrayQueue<KeyEvent> = ArrayQueue::new(1000);
  ```
- [ ] **Memory usage optimization**
  - Pool allocations for frequent operations
  - Minimize string allocations in hot paths
- [ ] **IPC performance tuning**
  - Message batching for high-frequency events
  - Async processing where appropriate

---

### **Phase 2: Architecture Refinement** *(Weeks 4-6)*

#### 2.1 Simplified IPC Communication
```rust
// Priority: MEDIUM
// Goal: Replace named pipes with simpler communication
```

**Current Problem:**
```rust
// Named pipes are complex for simple messaging
let listener = LocalSocketListener::bind(pipe_name)?;
```

**Proposed Solution:**
```rust
// Use memory-mapped files or simple file-based messaging
pub struct SimpleIpc {
    command_file: PathBuf,
    response_file: PathBuf,
}

impl SimpleIpc {
    fn send_command(&self, cmd: &IpcMessage) -> Result<()> {
        let json = serde_json::to_string(cmd)?;
        fs::write(&self.command_file, json)?;
        Ok(())
    }
    
    fn poll_commands(&self) -> Result<Option<IpcMessage>> {
        if self.command_file.exists() {
            let content = fs::read_to_string(&self.command_file)?;
            fs::remove_file(&self.command_file)?;
            Ok(Some(serde_json::from_str(&content)?))
        } else {
            Ok(None)
        }
    }
}
```

**Tasks:**
- [ ] **Implement file-based IPC**
- [ ] **Benchmark IPC performance** (named pipes vs file-based vs shared memory)
- [ ] **Add IPC health monitoring**
- [ ] **Graceful IPC failure handling**

#### 2.2 Keyboard Hook Redesign
```rust
// Priority: HIGH
// Goal: More robust keyboard monitoring
```

**Current Issues:**
- Direct Windows API calls are brittle
- No separation between detection and processing
- Poor error recovery

**Proposed Architecture:**
```rust
pub trait KeyboardMonitor {
    fn start(&mut self, callback: Box<dyn Fn(KeyEvent) + Send>) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn is_running(&self) -> bool;
}

pub struct WindowsKeyboardMonitor {
    hook_handle: Option<HHOOK>,
    callback: Option<Box<dyn Fn(KeyEvent) + Send>>,
    error_count: AtomicU32,
}

impl KeyboardMonitor for WindowsKeyboardMonitor {
    fn start(&mut self, callback: Box<dyn Fn(KeyEvent) + Send>) -> Result<()> {
        // Implement with retry logic and error recovery
    }
}
```

**Tasks:**
- [ ] **Abstract keyboard monitoring interface**
- [ ] **Implement retry logic for failed hooks**
- [ ] **Add keyboard hook health monitoring**
- [ ] **Separate key detection from text processing**

#### 2.3 Configuration System Enhancement
```rust
// Priority: MEDIUM
// Goal: More flexible and robust configuration
```

**Tasks:**
- [ ] **Hot-reloading without restart**
  ```rust
  pub struct ConfigWatcher {
      watcher: RecommendedWatcher,
      config_cache: Arc<RwLock<TextraConfig>>,
  }
  ```
- [ ] **Configuration validation**
  ```rust
  impl TextraConfig {
      pub fn validate(&self) -> Result<(), ConfigError> {
          // Validate triggers don't conflict
          // Validate replacements are valid
          // Check for circular references
      }
  }
  ```
- [ ] **Migration system for config changes**
- [ ] **Backup and restore functionality**

---

### **Phase 3: Feature Enhancement** *(Weeks 7-10)*

#### 3.1 Advanced Text Processing
```rust
// Priority: MEDIUM
// Goal: More sophisticated text expansion
```

**New Features:**
- [ ] **Context-aware replacements**
  ```rust
  pub struct ContextRule {
      trigger: String,
      replacement: String,
      window_class_filter: Option<Regex>,
      application_filter: Option<String>,
      time_filter: Option<TimeRange>,
  }
  ```
- [ ] **Variable interpolation**
  ```rust
  // Support for {{clipboard}}, {{selection}}, {{app_name}}
  pub fn process_variables(text: &str, context: &AppContext) -> String {
      text.replace("{{clipboard}}", &context.clipboard)
          .replace("{{app_name}}", &context.current_app)
  }
  ```
- [ ] **Conditional replacements**
  ```rust
  // IF conditions in replacements
  {{IF app_name == "notepad"}}formal_greeting{{ELSE}}casual_greeting{{END}}
  ```

#### 3.2 Enhanced Overlay System
```rust
// Priority: MEDIUM
// Goal: Lighter, more responsive overlay
```

**Current Issue:** Web-view is heavy for simple template selection

**Proposed Solution:**
```rust
// Option 1: Native Windows overlay using raw Windows APIs
pub struct NativeOverlay {
    hwnd: HWND,
    templates: Vec<Template>,
    selected_index: usize,
}

// Option 2: Immediate mode GUI with egui
pub struct EguiOverlay {
    ctx: egui::Context,
    templates: Vec<Template>,
}
```

**Tasks:**
- [ ] **Prototype native Windows overlay**
- [ ] **Benchmark overlay performance** (web-view vs native vs egui)
- [ ] **Implement keyboard navigation**
- [ ] **Add search functionality**
- [ ] **Theming support**

#### 3.3 Smart Suggestions
```rust
// Priority: LOW
// Goal: AI-powered text suggestions
```

**Tasks:**
- [ ] **Frequency-based suggestions**
  ```rust
  pub struct UsageTracker {
      trigger_frequency: HashMap<String, u32>,
      last_used: HashMap<String, SystemTime>,
  }
  ```
- [ ] **Context prediction**
- [ ] **Learning from user patterns**

---

### **Phase 4: Polish & Distribution** *(Weeks 11-12)*

#### 4.1 User Experience Improvements
- [ ] **Better installation experience**
  ```powershell
  # One-liner installation
  irm https://textra.com/install.ps1 | iex
  ```
- [ ] **Configuration GUI**
- [ ] **Better error messages for users**
- [ ] **Comprehensive documentation**

#### 4.2 Security & Reliability
- [ ] **Code signing for executables**
- [ ] **Privilege escalation handling**
- [ ] **Anti-virus compatibility testing**
- [ ] **Memory safety audit**

#### 4.3 Distribution & Updates
- [ ] **Automated release pipeline**
- [ ] **Delta updates** (only download changed parts)
- [ ] **Rollback capability**
- [ ] **Telemetry for crash reporting** (opt-in)

---

## 🛠️ Technical Implementation Details

### **Error Handling Strategy**
```rust
// Global error handling approach
pub type Result<T> = std::result::Result<T, TextraError>;

// Context-preserving error chains
use anyhow::Context;
let config = load_config()
    .context("Failed to load configuration")
    .context("During application startup")?;
```

### **Performance Monitoring**
```rust
// Built-in performance tracking
use std::time::Instant;

pub struct PerformanceTracker {
    key_processing_times: VecDeque<Duration>,
    replacement_times: VecDeque<Duration>,
}

impl PerformanceTracker {
    pub fn track_key_processing<F, R>(&mut self, f: F) -> R 
    where F: FnOnce() -> R {
        let start = Instant::now();
        let result = f();
        self.key_processing_times.push_back(start.elapsed());
        result
    }
}
```

### **Resource Management**
```rust
// RAII for Windows resources
pub struct KeyboardHook {
    handle: HHOOK,
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.handle);
        }
    }
}
```

---

## 📋 Implementation Checklist

### **Week-by-Week Breakdown**

#### **Week 1: Foundation**
- [ ] Set up comprehensive logging with `tracing`
- [ ] Replace all `unwrap()` calls in `textra-core`
- [ ] Implement `TextraError` enum with proper error chains
- [ ] Set up basic unit test framework

#### **Week 2: Testing Infrastructure**
- [ ] Create mock Windows APIs for testing
- [ ] Implement integration tests for IPC
- [ ] Set up GitHub Actions CI/CD
- [ ] Add performance benchmarks

#### **Week 3: Performance Optimization**
- [ ] Optimize keyboard processing hot path
- [ ] Implement lock-free data structures for key buffer
- [ ] Profile memory usage and optimize allocations
- [ ] Add performance monitoring

#### **Week 4-5: IPC Redesign**
- [ ] Implement file-based IPC alternative
- [ ] Benchmark different IPC approaches
- [ ] Migrate to simpler IPC solution
- [ ] Add IPC health monitoring

#### **Week 6: Keyboard Hook Redesign**
- [ ] Abstract keyboard monitoring interface
- [ ] Implement retry logic and error recovery
- [ ] Add keyboard hook health monitoring
- [ ] Separate detection from processing

#### **Week 7-8: Advanced Features**
- [ ] Implement context-aware replacements
- [ ] Add variable interpolation support
- [ ] Create conditional replacement system
- [ ] Add usage tracking and analytics

#### **Week 9-10: UI Enhancement**
- [ ] Prototype native overlay
- [ ] Implement search functionality
- [ ] Add theming support
- [ ] Optimize overlay performance

#### **Week 11-12: Polish & Release**
- [ ] Create installation GUI
- [ ] Implement automated updates
- [ ] Add comprehensive documentation
- [ ] Security audit and code signing

---

## 🎯 Success Metrics

### **Performance Targets**
- **Key processing latency**: < 1ms average
- **Memory usage**: < 50MB resident
- **CPU usage**: < 2% when idle
- **Startup time**: < 500ms

### **Reliability Targets**
- **Uptime**: > 99.9% (less than 1 crash per 1000 hours)
- **Error recovery**: Automatic recovery from 95% of transient failures
- **Test coverage**: > 80% line coverage

### **User Experience Targets**
- **Installation time**: < 30 seconds
- **Configuration changes**: Applied without restart
- **Overlay response time**: < 100ms to show
- **Documentation**: Complete with examples

---

## 🚀 Getting Started

### **Immediate Next Steps**
1. **Set up development environment**
   ```bash
   git clone your-repo
   cd textra
   cargo test  # Should pass basic tests
   ```

2. **Start with Week 1 tasks**
   - Focus on error handling in `src/bin/core.rs`
   - Replace `unwrap()` calls one by one
   - Add structured logging

3. **Create development branch structure**
   ```bash
   git checkout -b phase1-foundation
   git checkout -b phase2-architecture  
   git checkout -b phase3-features
   git checkout -b phase4-polish
   ```

This plan provides a clear roadmap from your current implementation to a production-ready, robust text expansion tool. Each phase builds on the previous one while maintaining backward compatibility and user functionality.