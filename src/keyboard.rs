pub use super::*;


pub struct AppState {
    config: Arc<Mutex<Config>>,
    current_text: Arc<Mutex<Rope>>,
    last_key_time: Arc<Mutex<Instant>>,
    shift_pressed: Arc<AtomicBool>,
    caps_lock_on: Arc<AtomicBool>,
    killswitch: Arc<AtomicBool>,
}

impl AppState {
    pub fn new() -> Result<Self> {
        let config = config::load_config().unwrap_or_else(|e| {
            eprintln!("Error loading config: {}", e);
            config::Config::default()
        });

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            current_text: Arc::new(Mutex::new(Rope::new())),
            last_key_time: Arc::new(Mutex::new(Instant::now())),
            shift_pressed: Arc::new(AtomicBool::new(false)),
            caps_lock_on: Arc::new(AtomicBool::new(false)),
            killswitch: Arc::new(AtomicBool::new(false)),
        })
    }
}


pub fn main_loop(app_state: Arc<AppState>, receiver: &Receiver<Message>) -> Result<()> {
while let Ok(msg) = receiver.recv() {
    match msg {
        Message::KeyEvent(vk_code, w_param, l_param) => {
            if let Err(e) = handle_key_event(Arc::clone(&app_state), vk_code, w_param, l_param)
            {
                eprintln!("Error handling key event: {}", e);
            }
        }
        Message::ConfigReload => {
            if let Err(e) = reload_config(Arc::clone(&app_state)) {
                eprintln!("Error reloading config: {}", e);
            }
        }
        Message::Quit => break,
    }
}
Ok(())
}

unsafe extern "system" fn keyboard_hook_proc(
code: i32,
w_param: WPARAM,
l_param: LPARAM,
) -> LRESULT {
if code >= 0 && !listener_is_blocked() {
    let kb_struct = *(l_param as *const KBDLLHOOKSTRUCT);
    let vk_code = kb_struct.vkCode;

    if let Some(sender) = config::GLOBAL_SENDER.lock().as_ref() {
        let _ = sender.try_send(Message::KeyEvent(vk_code, w_param, l_param));
    }
}

CallNextHookEx(ptr::null_mut(), code, w_param, l_param)
}

pub fn listen_keyboard(sender: Sender<Message>) -> Result<()> {
config::set_global_sender(sender);
unsafe {
    let hook = SetWindowsHookExA(WH_KEYBOARD_LL, Some(keyboard_hook_proc), ptr::null_mut(), 0);
    if hook.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    let mut msg: MSG = mem::zeroed();
    while GetMessageA(&mut msg, ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
    UnhookWindowsHookEx(hook);
}
Ok(())
}

lazy_static::lazy_static! {
static ref SYMBOL_PAIRS: HashMap<char, char> = {
    let mut m = HashMap::new();
    m.insert(';', ':');
    m.insert(',', '<');
    m.insert('.', '>');
    m.insert('/', '?');
    m.insert('\'', '"');
    m.insert('[', '{');
    m.insert(']', '}');
    m.insert('\\', '|');
    m.insert('`', '~');
    m.insert('1', '!');
    m.insert('2', '@');
    m.insert('3', '#');
    m.insert('4', '$');
    m.insert('5', '%');
    m.insert('6', '^');
    m.insert('7', '&');
    m.insert('8', '*');
    m.insert('9', '(');
    m.insert('0', ')');
    m.insert('-', '_');
    m.insert('=', '+');
    m
};
}

fn handle_key_event(
app_state: Arc<AppState>,
vk_code: DWORD,
w_param: WPARAM,
l_param: LPARAM,
) -> Result<()> {
let now = Instant::now();

match w_param as u32 {
    WM_KEYDOWN | WM_SYSKEYDOWN => {
        let mut last_key_time = app_state.last_key_time.lock();
        if now.duration_since(*last_key_time) > Duration::from_millis(1000) {
            app_state.current_text.lock().remove(..);
        }
        *last_key_time = now;

        match vk_code as i32 {
            VK_ESCAPE => {
                app_state.killswitch.store(true, Ordering::SeqCst);
            }
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => {
                app_state.shift_pressed.store(true, Ordering::SeqCst);
            }
            VK_CAPITAL => {
                let current = app_state.caps_lock_on.load(Ordering::SeqCst);
                app_state.caps_lock_on.store(!current, Ordering::SeqCst);
            }
            _ => {
                if let Some(c) = get_char_from_vk(
                    vk_code as i32,
                    app_state.shift_pressed.load(Ordering::SeqCst),
                ) {
                    let mut current_text = app_state.current_text.lock();
                    let txtlen = current_text.len_chars();
                    current_text.insert(txtlen, &c.to_string());
                    check_and_replace(&app_state, &mut current_text)?;
                }
            }
        }
    }
    WM_KEYUP | WM_SYSKEYUP => match vk_code as i32 {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => {
            app_state.shift_pressed.store(false, Ordering::SeqCst);
        }
        VK_ESCAPE => {
            app_state.killswitch.store(false, Ordering::SeqCst);
        }
        _ => {}
    },
    _ => {}
}

Ok(())
}

fn get_char_from_vk(vk_code: i32, shift_pressed: bool) -> Option<char> {
unsafe {
    let mut keyboard_state: [u8; 256] = std::mem::zeroed();
    if shift_pressed {
        keyboard_state[VK_SHIFT as usize] = 0x80;
    }
    GetKeyboardState(keyboard_state.as_mut_ptr());

    let scan_code =
        MapVirtualKeyExW(vk_code as u32, MAPVK_VK_TO_VSC_EX, ptr::null_mut()) as u16;
    let mut char_buffer: [u16; 2] = [0; 2];

    let result = ToUnicodeEx(
        vk_code as u32,
        scan_code as u32,
        keyboard_state.as_ptr(),
        char_buffer.as_mut_ptr(),
        2,
        0,
        GetKeyboardLayout(0),
    );

    if result == 1 {
        let c = char::from_u32(char_buffer[0] as u32)?;
        if shift_pressed {
            SYMBOL_PAIRS.get(&c).cloned().or(Some(c))
        } else {
            Some(c)
        }
    } else {
        None
    }
}
}

fn check_and_replace(app_state: &AppState, current_text: &mut Rope) -> Result<()> {
let immutable_current_text = current_text.to_string();
let config = app_state.config.lock();

for match_rule in &config.matches {
    match &match_rule.replacement {
        Replacement::Static {
            text,
            propagate_case,
        } => {
            if immutable_current_text.ends_with(&match_rule.trigger) {
                let start = immutable_current_text.len() - match_rule.trigger.len();

                perform_replacement(
                    current_text,
                    config.backend.key_delay,
                    &immutable_current_text[start..],
                    text,
                    *propagate_case,
                    false,
                    app_state,
                )?;

                return Ok(());
            }
        }
        Replacement::Dynamic { action } => {
            if immutable_current_text.ends_with(&match_rule.trigger) {
                let replacement = process_dynamic_replacement(action);

                perform_replacement(
                    current_text,
                    config.backend.key_delay,
                    &match_rule.trigger,
                    &replacement,
                    false,
                    true,
                    app_state,
                )?;

                return Ok(());
            }
        }
    }

    if match_rule.trigger.starts_with("regex") && match_rule.trigger.len() > 6 {
        let regex = Regex::new(&match_rule.trigger[5..]).unwrap();
        if let Some(captures) = regex.captures(&immutable_current_text) {
            let replacement = match &match_rule.replacement {
                Replacement::Static { text, .. } => text.clone(),
                Replacement::Dynamic { action } => process_dynamic_replacement(action),
            };
            let mut final_replacement = replacement.clone();
            for (i, capture) in captures.iter().enumerate().skip(1) {
                if let Some(capture) = capture {
                    final_replacement =
                        final_replacement.replace(&format!("${}", i), capture.as_str());
                }
            }
            perform_replacement(
                current_text,
                config.backend.key_delay,
                &immutable_current_text,
                &final_replacement,
                false,
                false,
                app_state,
            )?;
            return Ok(());
        }
    }
}
Ok(())
}

fn perform_replacement(
current_text: &mut Rope,
key_delay: u64,
original: &str,
replacement: &str,
propagate_case: bool,
dynamic: bool,
app_state: &AppState,
) -> Result<()> {
let final_replacement = if dynamic {
    process_dynamic_replacement(replacement)
} else if propagate_case {
    propagate_case_fn(original, replacement)
} else {
    replacement.to_string()
};

if app_state.killswitch.load(Ordering::SeqCst) {
    return Ok(());
}

// Block the listener before simulating key presses
block_listener();

// Backspace the original text
let backspace_count = original.chars().count();
let backspaces = vec![VK_BACK; backspace_count];
simulate_key_presses(&backspaces, key_delay);

// Type the replacement
let vk_codes = string_to_vk_codes(&final_replacement);
simulate_key_presses(&vk_codes, key_delay);

// Unblock the listener after simulating key presses
unblock_listener();

let start = current_text.len_chars() - original.len();
current_text.remove(start..current_text.len_chars());
current_text.insert(start, &final_replacement);

Ok(())
}

fn process_dynamic_replacement(replacement: &str) -> String {
match replacement.to_lowercase().as_str() {
    "{{date}}" => Local::now().format("%Y-%m-%d").to_string(),
    _ => replacement.to_string(),
}
}

fn propagate_case_fn(original: &str, replacement: &str) -> String {
if original.chars().all(|c| c.is_uppercase()) {
    replacement.to_uppercase()
} else if original.chars().next().map_or(false, |c| c.is_uppercase()) {
    let mut chars = replacement.chars();
    match chars.next() {
        None => String::new(),
        Some(first_char) => first_char.to_uppercase().collect::<String>() + chars.as_str(),
    }
} else {
    replacement.to_string()
}
}

fn reload_config(app_state: Arc<AppState>) -> Result<()> {
let mut config = app_state.config.lock();
*config = config::load_config()?;
Ok(())
}

fn simulate_key_presses(vk_codes: &[i32], key_delay: u64) {
let batch_size = 20;
let delay = Duration::from_millis(key_delay);
let input_count = vk_codes.len() * 2;
let mut inputs: Vec<MaybeUninit<INPUT>> = Vec::with_capacity(input_count);

// Pre-allocate the entire vector
unsafe { inputs.set_len(input_count) };

// Fill the vector without initializing
for (i, &vk) in vk_codes.iter().enumerate() {
    let press_index = i * 2;
    let release_index = press_index + 1;

    // Key press event
    unsafe {
        let input = inputs[press_index].as_mut_ptr();
        (*input).type_ = INPUT_KEYBOARD;
        let ki = (*input).u.ki_mut();
        ki.wVk = vk as u16;
        ki.dwFlags = 0;
    }

    // Key release event
    unsafe {
        let input = inputs[release_index].as_mut_ptr();
        (*input).type_ = INPUT_KEYBOARD;
        let ki = (*input).u.ki_mut();
        ki.wVk = vk as u16;
        ki.dwFlags = KEYEVENTF_KEYUP;
    }
}

// Send inputs in batches
for chunk in inputs.chunks(batch_size * 2) {
    unsafe {
        SendInput(
            chunk.len() as u32,
            chunk.as_ptr() as *mut INPUT,
            std::mem::size_of::<INPUT>() as c_int,
        );
    }
    // Add a small delay between batches for more natural input
    thread::sleep(delay);
}
}

fn string_to_vk_codes(s: &str) -> Vec<i32> {
s.chars().map(|c| char_to_vk_code(c)).collect()
}

fn char_to_vk_code(c: char) -> i32 {
let vk_code = unsafe { VkKeyScanW(c as u16) as i32 };
let low_byte = vk_code & 0xFF;
let high_byte = (vk_code >> 8) & 0xFF;

if high_byte & 1 != 0 {
    // Shift is required
    VK_SHIFT
} else {
    low_byte
}
}



fn listener_is_blocked() -> bool {
GENERATING.load(Ordering::SeqCst)
}

fn block_listener() {
GENERATING.store(true, Ordering::SeqCst);
}

fn unblock_listener() {
GENERATING.store(false, Ordering::SeqCst);
}

lazy_static::lazy_static! {
static ref GENERATING: AtomicBool = AtomicBool::new(false);
}
