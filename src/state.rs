use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use enigo::Enigo;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use mlua::Function;
use nusb::DeviceId;
use nusb::DeviceInfo;

pub struct WrappedHotKeyManager(pub GlobalHotKeyManager);
#[cfg(target_os = "windows")]
unsafe impl Send for WrappedHotKeyManager {}
#[cfg(target_os = "windows")]
unsafe impl Sync for WrappedHotKeyManager {}

pub struct WrappedEnigo(pub Enigo);
#[cfg(target_os = "windows")]
unsafe impl Send for WrappedEnigo {}
#[cfg(target_os = "windows")]
unsafe impl Sync for WrappedEnigo {}

#[derive(Clone)]
pub struct AppState {
  pub hotkeys: Arc<Mutex<HashMap<HotKey, Function>>>,
  pub hk_manager: Arc<Mutex<WrappedHotKeyManager>>,
  pub hotplug: Arc<Mutex<Option<Function>>>,
  pub screen_edge: Arc<Mutex<Option<Function>>>,
  pub interval_callbacks: Arc<Mutex<HashMap<usize, (Function, (u64, u64), std::time::Instant)>>>,
  pub devices: Arc<Mutex<HashMap<DeviceId, DeviceInfo>>>,
  pub enigo: Arc<Mutex<WrappedEnigo>>,
}

pub static DO_MAIN_LOOP: AtomicBool = AtomicBool::new(false);

static INTERVAL_COUNTER: AtomicUsize = AtomicUsize::new(1);
pub fn get_interval_id() -> usize {
  INTERVAL_COUNTER.fetch_add(1, Ordering::Relaxed)
}
