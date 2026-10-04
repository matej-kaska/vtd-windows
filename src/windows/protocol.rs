//! Pointer-free messages shared by the small resident and transient helpers.
#![allow(dead_code)] // Each executable uses a different half of this protocol.

pub const COPY: u32 = 0x8001;
pub const KEY: u32 = 0x8002;
pub const PAUSE: u32 = 0x8005;
pub const SETTINGS: u32 = 0x8006;
pub const SETTINGS_READY: u32 = 0x8007;
pub const SETTINGS_APPLY: u32 = 0x8008;
pub const UI_CLOSED: u32 = 0x8009;
pub const MENU_READY: u32 = 0x800a;
pub const MENU_COMMAND: u32 = 0x800b;
pub const OPEN_SETTINGS: u32 = 0x800c;
pub const UI_CHECK: u32 = 0x800d;
pub const SESSION_READY: u32 = 0x800e;
pub const SESSION_CLOSED: u32 = 0x800f;
pub const GET_EPOCH: u32 = 0x8010;
pub const LAST_REQUEST: u32 = 0x8011;
pub const IDLE: u32 = 0x8012;
pub const BOOTSTRAP_CLOSED: u32 = 0x8013;
pub const TEST_KEY: u32 = 0x8014;
pub const TEST_HOOK_THREAD: u32 = 0x8015;
pub const SESSION_IDLE: u32 = 0x8016;
pub const KEY_ACK: u32 = 0x8017;
pub const CAPTURE_CONSUMED: u32 = 0x8018;
// Only resident-test builds accept these injected fixture keystrokes as shortcuts.
pub const TEST_INPUT_MARKER: usize = 0x5654_4449;

pub const PREFERENCES: usize = 0x5654_4401;
pub const STATUS: usize = 0x5654_4402;
pub const TRANSCRIPT: usize = 0x5654_4403;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Preferences {
    pub trigger: u32,
    pub toggle: u32,
    pub replay: u32,
}

#[repr(C)]
pub struct Status {
    pub active: u32,
    pub paused: u32,
    pub text: [u16; 128],
}

pub fn shortcut_matches(shortcut: u32, key: u32, modifiers: u32, held: bool, up: bool) -> bool {
    shortcut & 0xff == key && (held || (!up && shortcut & 0x700 == modifiers))
}
