#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use std::collections::HashMap;
use std::io::IsTerminal;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use clap::Parser;
use device_query::DeviceQuery;
use directories::ProjectDirs;
use display_info::DisplayInfo;
use errors::StdError;
use global_hotkey::GlobalHotKeyEvent;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::HotKeyState;
use global_hotkey::hotkey::HotKey;
use mlua::ExternalError;
use mlua::Function;
use mlua::Lua;
use mouse_position::mouse_position::Mouse;
use nusb::MaybeFuture;
use nusb::hotplug::HotplugEvent;
use rfd::MessageButtons;
use rfd::MessageDialog;
use rfd::MessageLevel;
use enigo::{Enigo, Mouse as EnigoMouse, Keyboard as EnigoKeyboard, Settings, Coordinate, Key, Direction};
use tao::event_loop::ControlFlow;
use tao::event_loop::EventLoop;
use tracing::Level;
use tracing::event;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod device;
mod errors;

static INTERVAL_COUNTER: AtomicUsize = AtomicUsize::new(1);

fn get_interval_id() -> usize {
  INTERVAL_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// Returns "n", "s", "w", "e", "ne", "nw", "se", "sw" if the mouse is at a
// screen edge/corner where it cannot move further in that direction across
// any of the displays. Returns None otherwise.
fn find_screen_edge(displays: &[DisplayInfo], x: i32, y: i32) -> Option<&'static str> {
  let in_display = |px: i32, py: i32| -> bool {
    displays.iter().any(|d| {
      let dw = d.width as i32;
      let dh = d.height as i32;
      px >= d.x && px < d.x + dw && py >= d.y && py < d.y + dh
    })
  };

  if !in_display(x, y) {
    return None;
  }

  let at_left = !in_display(x - 1, y);
  let at_right = !in_display(x + 1, y);
  let at_top = !in_display(x, y - 1);
  let at_bottom = !in_display(x, y + 1);

  match (at_top, at_bottom, at_left, at_right) {
    (true, _, true, _) => Some("nw"),
    (true, _, _, true) => Some("ne"),
    (_, true, true, _) => Some("sw"),
    (_, true, _, true) => Some("se"),
    (true, ..) => Some("n"),
    (_, true, ..) => Some("s"),
    (_, _, true, _) => Some("w"),
    (_, _, _, true) => Some("e"),
    _ => None,
  }
}

// We wrap GlobalHotKeyManager so we can send it across threads. This is
// safe to do for specific windows pointers like HWND since
// it is unique globally.
struct WrappedHotKeyManager(GlobalHotKeyManager);

#[cfg(target_os = "windows")]
unsafe impl Send for WrappedHotKeyManager {}
#[cfg(target_os = "windows")]
unsafe impl Sync for WrappedHotKeyManager {}

// We wrap Enigo so we can send it across threads. This is
// safe to do for specific windows pointers like HWND since
// it is unique globally.
struct WrappedEnigo(Enigo);

#[cfg(target_os = "windows")]
unsafe impl Send for WrappedEnigo {}
#[cfg(target_os = "windows")]
unsafe impl Sync for WrappedEnigo {}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  #[arg(short, long)]
  cmd: String,
  #[arg(long)]
  console: bool,
  #[arg(long)]
  cwd: Option<String>,
}

impl mlua::UserData for device::MSIDevice {
  // fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
  //   // fields.add_field_method_get("val", |_, this| Ok(this.0));
  //   fields.add_field_method_get("code", |_, this| Ok(Code));
  // }

  fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
    // methods.add_method_mut("get_volume", |_, this, ()| -> Result<u32, mlua::Error> {
    //   let val = this
    //     .get_volume()
    //     .map_err(mlua::ExternalError::into_lua_err)?;
    //   Ok(val)
    // });

    methods.add_method_mut("get_kvm", |_, this, ()| -> Result<u32, mlua::Error> {
      let val = this.get_kvm().map_err(mlua::ExternalError::into_lua_err)?;
      Ok(val)
    });

    methods.add_method_mut("get_input", |_, this, ()| -> Result<u32, mlua::Error> {
      let val = this
        .get_input()
        .map_err(mlua::ExternalError::into_lua_err)?;
      Ok(val)
    });

    // methods.add_method_mut(
    //   "set_volume",
    //   |_, this, level: u8| -> Result<(), mlua::Error> {
    //     this
    //       .set_volume(level)
    //       .map_err(mlua::ExternalError::into_lua_err)?;
    //     Ok(())
    //   },
    // );

    methods.add_method_mut(
      "set_kvm",
      |_, this, position: u8| -> Result<(), mlua::Error> {
        this
          .set_kvm(position)
          .map_err(mlua::ExternalError::into_lua_err)?;
        Ok(())
      },
    );

    methods.add_method_mut(
      "set_input",
      |_, this, position: u8| -> Result<(), mlua::Error> {
        this
          .set_input(position)
          .map_err(mlua::ExternalError::into_lua_err)?;
        Ok(())
      },
    );
  }
}

fn run() -> Result<(), Box<StdError>> {
  // let _ = std::process::Command::new("cmd.exe")
  //   .arg("/c")
  //   .arg("pause")
  //   .status();

  // use ddc::Ddc;
  // use ddc_winapi::Monitor;
  // for mut ddc in Monitor::enumerate().unwrap() {
  //   println!("{}", ddc.description());
  //   let s = ddc.capabilities_string().unwrap();
  //   println!("{}", String::from_utf8_lossy(&s));
  //   let v = ddc.get_vcp_feature(0xe3).unwrap();
  //   println!("{:?}", v);
  // }

  // let mut dev = device::MSIDevice::open(0x1462, 0x3fa4)?;
  // dev.test()?;

  // return Ok(());

  let args = Args::try_parse()?;

  if let Some(cwd) = args.cwd {
    std::env::set_current_dir(cwd)?;
  }

  #[cfg(target_os = "windows")]
  if args.console {
    unsafe {
      windows::Win32::System::Console::AllocConsole()?;
    }
  }

  let event_loop = EventLoop::new();

  let lua = Lua::new();

  let hotkeys_manager = GlobalHotKeyManager::new()?;
  let global_hotkey_channel = GlobalHotKeyEvent::receiver();
  let hotplug = Arc::new(Mutex::new(None));

  let (hotplug_tx, hotplug_rx) = crossbeam_channel::unbounded();
  std::thread::spawn(move || {
    let watch = nusb::watch_devices().unwrap();
    for event in futures_lite::stream::block_on(watch) {
      hotplug_tx.send(event).unwrap();
    }
  });

  let device_open = lua.create_function(
    |_, (vendor_id, product_id): (u16, u16)| -> Result<device::MSIDevice, mlua::Error> {
      let dev = device::MSIDevice::open(vendor_id, product_id)
        .map_err(mlua::ExternalError::into_lua_err)?;
      Ok(dev)
    },
  )?;

  let device_is_connected = lua.create_function(
    |_, (vendor_id, product_id): (u16, u16)| -> Result<bool, mlua::Error> {
      let connected = device::MSIDevice::is_connected(vendor_id, product_id)
        .map_err(mlua::ExternalError::into_lua_err)?;
      Ok(connected)
    },
  )?;

  let sleep_ms = lua.create_function(|_, duration: u64| -> Result<(), mlua::Error> {
    thread::sleep(Duration::from_millis(duration));
    Ok(())
  })?;

  let hotkeys = Arc::new(Mutex::new(HashMap::new()));

  let hotkeys_clone = hotkeys.clone();
  let hk_manager = Arc::new(Mutex::new(WrappedHotKeyManager(hotkeys_manager)));
  let register_hotkey = lua.create_function(
    move |_, (keybind, callback): (String, Function)| -> Result<(), mlua::Error> {
      let hotkey = HotKey::from_str(&keybind).map_err(mlua::ExternalError::into_lua_err)?;

      let hk_manager = hk_manager.lock().unwrap();
      hk_manager
        .0
        .register(hotkey)
        .map_err(mlua::ExternalError::into_lua_err)?;

      let mut hk = hotkeys_clone.lock().unwrap();
      hk.insert(hotkey, callback);
      Ok(())
    },
  )?;

  let hotplug_clone = hotplug.clone();
  let register_hotplug =
    lua.create_function(move |_, callback: Function| -> Result<(), mlua::Error> {
      let mut hp = hotplug_clone.lock().unwrap();
      *hp = Some(callback);
      Ok(())
    })?;

  let screen_edge: Arc<Mutex<Option<Function>>> = Arc::new(Mutex::new(None));
  let screen_edge_clone = screen_edge.clone();
  let register_screen_edge =
    lua.create_function(move |_, callback: Function| -> Result<(), mlua::Error> {
      let mut se = screen_edge_clone.lock().unwrap();
      *se = Some(callback);
      Ok(())
    })?;

  let interval_callbacks = Arc::new(Mutex::new(HashMap::new()));
  let interval_callbacks_clone = interval_callbacks.clone();
  let register_interval = lua.create_function(
    move |_,
          (lo_interval, hi_interval, callback): (u64, u64, Function)|
          -> Result<usize, mlua::Error> {
      if lo_interval > hi_interval {
        return Err(mlua::Error::external("lo_interval must be <= hi_interval"));
      }

      let mut ic = interval_callbacks_clone.lock().unwrap();
      let id = get_interval_id();
      let interval = rand::random_range(lo_interval..=hi_interval);
      let next = std::time::Instant::now() + Duration::from_millis(interval);
      ic.insert(id, (callback, (lo_interval, hi_interval), next));
      Ok(id)
    },
  )?;

  let interval_callbacks_clone = interval_callbacks.clone();
  let unregister_interval =
    lua.create_function(move |_, id: usize| -> Result<(), mlua::Error> {
      let mut ic = interval_callbacks_clone.lock().unwrap();
      ic.remove(&id);
      Ok(())
    })?;

  let msgbox = lua.create_function(
    move |_,
          (title, message, level, buttoncfg): (
      String,
      String,
      String,
      HashMap<String, Vec<String>>,
    )|
          -> Result<(), mlua::Error> {
      let level = match level.to_lowercase().as_str() {
        "info" | "" => MessageLevel::Info,
        "warning" => MessageLevel::Warning,
        "error" => MessageLevel::Error,
        _ => {
          return Err(mlua::Error::external(format!(
            "unknown msgbox level: {}",
            level
          )));
        },
      };

      let mut buttons = buttoncfg.into_iter();
      let button_opt = buttons.next().unwrap_or(("Ok".into(), vec![]));
      let button_opt_len = button_opt.1.len();
      let mut x = button_opt.1.into_iter();
      let btns = match (button_opt.0.as_str(), button_opt_len) {
        ("Ok", 0) => MessageButtons::Ok,
        ("Ok", 1) => MessageButtons::OkCustom(x.next().unwrap()),
        ("OkCancel", 0) => MessageButtons::OkCancel,
        ("OkCancel", 1) => MessageButtons::OkCancelCustom(x.next().unwrap(), "Cancel".into()),
        ("OkCancel", 2) => MessageButtons::OkCancelCustom(x.next().unwrap(), x.next().unwrap()),
        ("YesNo", 0) => MessageButtons::YesNo,
        ("YesNoCancel", 0) => MessageButtons::YesNoCancel,
        ("YesNoCancel", 1) => {
          MessageButtons::YesNoCancelCustom(x.next().unwrap(), "No".into(), "Cancel".into())
        },
        ("YesNoCancel", 2) => {
          MessageButtons::YesNoCancelCustom(x.next().unwrap(), x.next().unwrap(), "Cancel".into())
        },
        ("YesNoCancel", 3) => {
          MessageButtons::YesNoCancelCustom(x.next().unwrap(), x.next().unwrap(), x.next().unwrap())
        },
        (type_, _) => {
          return Err(mlua::Error::external(format!(
            "unknown msgbox buttons: {}={}",
            type_,
            x.as_slice().join(","),
          )));
        },
      };

      let dialog = MessageDialog::new()
        .set_title(title)
        .set_description(message)
        .set_buttons(btns)
        .set_level(level);
      dialog.show();
      Ok(())
    },
  )?;

  let devices: Arc<Mutex<HashMap<nusb::DeviceId, nusb::DeviceInfo>>> = Arc::new(Mutex::new(
    nusb::list_devices()
      .wait()
      .unwrap()
      .map(|d| (d.id(), d))
      .collect(),
  ));

  let autorun = lua.create_function(
    |_, (app_path, args): (Option<String>, Option<Vec<String>>)| -> Result<(), mlua::Error> {
      let mut autolaunch = auto_launch::AutoLaunchBuilder::new();

      autolaunch
        .set_app_name(env!("CARGO_CRATE_NAME"))
        .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);

      match (app_path, args) {
        (Some(app_path), Some(args)) => {
          autolaunch
            .set_app_path(app_path.as_ref())
            .set_args(args.as_ref());
        },
        (Some(app_path), None) => {
          autolaunch.set_app_path(app_path.as_ref()).set_args(
            &std::env::args()
              .skip(1)
              .map(|v| format!(r#""{}""#, v))
              .collect::<Vec<_>>(),
          );
        },
        (None, Some(args)) => {
          autolaunch
            .set_app_path(std::env::current_exe()?.to_str().unwrap())
            .set_args(args.as_ref());
        },
        (None, None) => {
          autolaunch
            .set_app_path(std::env::current_exe()?.to_str().unwrap())
            .set_args(
              &std::env::args()
                .skip(1)
                .map(|v| format!(r#""{}""#, v))
                .collect::<Vec<_>>(),
            );
        },
      };

      let autolaunch = autolaunch.build().map_err(|e| e.into_lua_err())?;
      autolaunch.enable().map_err(|e| e.into_lua_err())?;

      Ok(())
    },
  )?;

  let enigo_instance = Arc::new(Mutex::new(WrappedEnigo(
    Enigo::new(&Settings::default()).map_err(|e| e.into_lua_err())?,
  )));
  let enigo_clone = enigo_instance.clone();
  let screen_size = lua.create_function(move |_, ()| -> Result<(i32, i32), mlua::Error> {
    let enigo_guard = enigo_clone.lock().unwrap();
    enigo_guard.0.main_display().map_err(|e| e.into_lua_err())
  })?;
  let enigo_clone = enigo_instance.clone();
  let move_mouse = lua.create_function(
    move |_, (x, y, moving_time, mode): (i64, i64, f32, String)| -> Result<(), mlua::Error> {
      let moving_time = if moving_time > 10.0 { 10.0 } else { moving_time };

      let end_x = i32::try_from(x).map_err(|e| e.into_lua_err())?;
      let end_y = i32::try_from(y).map_err(|e| e.into_lua_err())?;

      let (start_x, start_y) = match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => (x, y),
        Mouse::Error => return Err(mlua::Error::external("could not get current mouse position")),
      };

      let (target_x, target_y) = match mode.as_str() {
        "rel" => (start_x + end_x, start_y + end_y),
        "abs" => (end_x, end_y),
        _ => return Err(mlua::Error::external(format!("unknown move_mouse mode: {}", mode))),
      };

      let mut enigo_guard = enigo_clone.lock().unwrap();
      if moving_time <= 0.0 {
        enigo_guard.0.move_mouse(target_x, target_y, Coordinate::Abs).map_err(|e| e.into_lua_err())?;
        return Ok(());
      }

      let start = std::time::Instant::now();
      let duration = Duration::from_secs_f32(moving_time);

      while start.elapsed() < duration {
        let progress = start.elapsed().as_secs_f32() / moving_time;
        let cur_x = start_x as f32 + (target_x as f32 - start_x as f32) * progress;
        let cur_y = start_y as f32 + (target_y as f32 - start_y as f32) * progress;
        
        enigo_guard.0.move_mouse(cur_x as i32, cur_y as i32, Coordinate::Abs).map_err(|e| e.into_lua_err())?;
        std::thread::sleep(Duration::from_millis(10));
      }
      
      enigo_guard.0.move_mouse(target_x, target_y, Coordinate::Abs).map_err(|e| e.into_lua_err())?;
      Ok(())
    },
  )?;

  let enigo_clone = enigo_instance.clone();
  let key = lua.create_function(
    move |_, (key_str, direction): (String, String)| -> Result<(), mlua::Error> {
      let parsed_key = match key_str.to_lowercase().as_str() {
#[cfg(target_os = "windows")]
        "num0" => Key::Num0,
        #[cfg(target_os = "windows")]
        "num1" => Key::Num1,
        #[cfg(target_os = "windows")]
        "num2" => Key::Num2,
        #[cfg(target_os = "windows")]
        "num3" => Key::Num3,
        #[cfg(target_os = "windows")]
        "num4" => Key::Num4,
        #[cfg(target_os = "windows")]
        "num5" => Key::Num5,
        #[cfg(target_os = "windows")]
        "num6" => Key::Num6,
        #[cfg(target_os = "windows")]
        "num7" => Key::Num7,
        #[cfg(target_os = "windows")]
        "num8" => Key::Num8,
        #[cfg(target_os = "windows")]
        "num9" => Key::Num9,
        #[cfg(target_os = "windows")]
        "a" => Key::A,
        #[cfg(target_os = "windows")]
        "b" => Key::B,
        #[cfg(target_os = "windows")]
        "c" => Key::C,
        #[cfg(target_os = "windows")]
        "d" => Key::D,
        #[cfg(target_os = "windows")]
        "e" => Key::E,
        #[cfg(target_os = "windows")]
        "f" => Key::F,
        #[cfg(target_os = "windows")]
        "g" => Key::G,
        #[cfg(target_os = "windows")]
        "h" => Key::H,
        #[cfg(target_os = "windows")]
        "i" => Key::I,
        #[cfg(target_os = "windows")]
        "j" => Key::J,
        #[cfg(target_os = "windows")]
        "k" => Key::K,
        #[cfg(target_os = "windows")]
        "l" => Key::L,
        #[cfg(target_os = "windows")]
        "m" => Key::M,
        #[cfg(target_os = "windows")]
        "n" => Key::N,
        #[cfg(target_os = "windows")]
        "o" => Key::O,
        #[cfg(target_os = "windows")]
        "p" => Key::P,
        #[cfg(target_os = "windows")]
        "q" => Key::Q,
        #[cfg(target_os = "windows")]
        "r" => Key::R,
        #[cfg(target_os = "windows")]
        "s" => Key::S,
        #[cfg(target_os = "windows")]
        "t" => Key::T,
        #[cfg(target_os = "windows")]
        "u" => Key::U,
        #[cfg(target_os = "windows")]
        "v" => Key::V,
        #[cfg(target_os = "windows")]
        "w" => Key::W,
        #[cfg(target_os = "windows")]
        "x" => Key::X,
        #[cfg(target_os = "windows")]
        "y" => Key::Y,
        #[cfg(target_os = "windows")]
        "z" => Key::Z,
        #[cfg(target_os = "windows")]
        "abntc1" => Key::AbntC1,
        #[cfg(target_os = "windows")]
        "abntc2" => Key::AbntC2,
        #[cfg(target_os = "windows")]
        "accept" => Key::Accept,
        "add" => Key::Add,
        "alt" => Key::Alt,
        #[cfg(target_os = "windows")]
        "apps" => Key::Apps,
        #[cfg(target_os = "windows")]
        "attn" => Key::Attn,
        "backspace" => Key::Backspace,
        #[cfg(all(unix, not(target_os = "macos")))]
        "break" => Key::Break,
        #[cfg(all(unix, not(target_os = "macos")))]
        "begin" => Key::Begin,
        #[cfg(target_os = "macos")]
        "brightnessdown" => Key::BrightnessDown,
        #[cfg(target_os = "macos")]
        "brightnessup" => Key::BrightnessUp,
        #[cfg(target_os = "windows")]
        "browserback" => Key::BrowserBack,
        #[cfg(target_os = "windows")]
        "browserfavorites" => Key::BrowserFavorites,
        #[cfg(target_os = "windows")]
        "browserforward" => Key::BrowserForward,
        #[cfg(target_os = "windows")]
        "browserhome" => Key::BrowserHome,
        #[cfg(target_os = "windows")]
        "browserrefresh" => Key::BrowserRefresh,
        #[cfg(target_os = "windows")]
        "browsersearch" => Key::BrowserSearch,
        #[cfg(target_os = "windows")]
        "browserstop" => Key::BrowserStop,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "cancel" => Key::Cancel,
        "capslock" => Key::CapsLock,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "clear" => Key::Clear,
        #[cfg(target_os = "macos")]
        "contrastup" => Key::ContrastUp,
        #[cfg(target_os = "macos")]
        "contrastdown" => Key::ContrastDown,
        "control" => Key::Control,
        #[cfg(target_os = "windows")]
        "convert" => Key::Convert,
        #[cfg(target_os = "windows")]
        "crsel" => Key::Crsel,
        #[cfg(target_os = "windows")]
        "dbealphanumeric" => Key::DBEAlphanumeric,
        #[cfg(target_os = "windows")]
        "dbecodeinput" => Key::DBECodeinput,
        #[cfg(target_os = "windows")]
        "dbedeterminestring" => Key::DBEDetermineString,
        #[cfg(target_os = "windows")]
        "dbeenterdlgconversionmode" => Key::DBEEnterDLGConversionMode,
        #[cfg(target_os = "windows")]
        "dbeenterimeconfigmode" => Key::DBEEnterIMEConfigMode,
        #[cfg(target_os = "windows")]
        "dbeenterwordregistermode" => Key::DBEEnterWordRegisterMode,
        #[cfg(target_os = "windows")]
        "dbeflushstring" => Key::DBEFlushString,
        #[cfg(target_os = "windows")]
        "dbehiragana" => Key::DBEHiragana,
        #[cfg(target_os = "windows")]
        "dbekatakana" => Key::DBEKatakana,
        #[cfg(target_os = "windows")]
        "dbenocodepoint" => Key::DBENoCodepoint,
        #[cfg(target_os = "windows")]
        "dbenoroman" => Key::DBENoRoman,
        #[cfg(target_os = "windows")]
        "dberoman" => Key::DBERoman,
        #[cfg(target_os = "windows")]
        "dbesbcschar" => Key::DBESBCSChar,
        #[cfg(target_os = "windows")]
        "dbeschar" => Key::DBESChar,
        "decimal" => Key::Decimal,
        "delete" => Key::Delete,
        "divide" => Key::Divide,
        "downarrow" => Key::DownArrow,
        #[cfg(target_os = "macos")]
        "eject" => Key::Eject,
        "end" => Key::End,
        #[cfg(target_os = "windows")]
        "ereof" => Key::Ereof,
        "escape" => Key::Escape,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "execute" => Key::Execute,
        #[cfg(target_os = "windows")]
        "exsel" => Key::Exsel,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "f13" => Key::F13,
        "f14" => Key::F14,
        "f15" => Key::F15,
        "f16" => Key::F16,
        "f17" => Key::F17,
        "f18" => Key::F18,
        "f19" => Key::F19,
        "f20" => Key::F20,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "f21" => Key::F21,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "f22" => Key::F22,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "f23" => Key::F23,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "f24" => Key::F24,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f25" => Key::F25,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f26" => Key::F26,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f27" => Key::F27,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f28" => Key::F28,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f29" => Key::F29,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f30" => Key::F30,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f31" => Key::F31,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f32" => Key::F32,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f33" => Key::F33,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f34" => Key::F34,
        #[cfg(all(unix, not(target_os = "macos")))]
        "f35" => Key::F35,
        #[cfg(target_os = "macos")]
        "function" => Key::Function,
        #[cfg(target_os = "windows")]
        "final" => Key::Final,
        #[cfg(all(unix, not(target_os = "macos")))]
        "find" => Key::Find,
        #[cfg(target_os = "windows")]
        "gamepada" => Key::GamepadA,
        #[cfg(target_os = "windows")]
        "gamepadb" => Key::GamepadB,
        #[cfg(target_os = "windows")]
        "gamepaddpaddown" => Key::GamepadDPadDown,
        #[cfg(target_os = "windows")]
        "gamepaddpadleft" => Key::GamepadDPadLeft,
        #[cfg(target_os = "windows")]
        "gamepaddpadright" => Key::GamepadDPadRight,
        #[cfg(target_os = "windows")]
        "gamepaddpadup" => Key::GamepadDPadUp,
        #[cfg(target_os = "windows")]
        "gamepadleftshoulder" => Key::GamepadLeftShoulder,
        #[cfg(target_os = "windows")]
        "gamepadleftthumbstickbutton" => Key::GamepadLeftThumbstickButton,
        #[cfg(target_os = "windows")]
        "gamepadleftthumbstickdown" => Key::GamepadLeftThumbstickDown,
        #[cfg(target_os = "windows")]
        "gamepadleftthumbstickleft" => Key::GamepadLeftThumbstickLeft,
        #[cfg(target_os = "windows")]
        "gamepadleftthumbstickright" => Key::GamepadLeftThumbstickRight,
        #[cfg(target_os = "windows")]
        "gamepadleftthumbstickup" => Key::GamepadLeftThumbstickUp,
        #[cfg(target_os = "windows")]
        "gamepadlefttrigger" => Key::GamepadLeftTrigger,
        #[cfg(target_os = "windows")]
        "gamepadmenu" => Key::GamepadMenu,
        #[cfg(target_os = "windows")]
        "gamepadrightshoulder" => Key::GamepadRightShoulder,
        #[cfg(target_os = "windows")]
        "gamepadrightthumbstickbutton" => Key::GamepadRightThumbstickButton,
        #[cfg(target_os = "windows")]
        "gamepadrightthumbstickdown" => Key::GamepadRightThumbstickDown,
        #[cfg(target_os = "windows")]
        "gamepadrightthumbstickleft" => Key::GamepadRightThumbstickLeft,
        #[cfg(target_os = "windows")]
        "gamepadrightthumbstickright" => Key::GamepadRightThumbstickRight,
        #[cfg(target_os = "windows")]
        "gamepadrightthumbstickup" => Key::GamepadRightThumbstickUp,
        #[cfg(target_os = "windows")]
        "gamepadrighttrigger" => Key::GamepadRightTrigger,
        #[cfg(target_os = "windows")]
        "gamepadview" => Key::GamepadView,
        #[cfg(target_os = "windows")]
        "gamepadx" => Key::GamepadX,
        #[cfg(target_os = "windows")]
        "gamepady" => Key::GamepadY,
        #[cfg(target_os = "windows")]
        "hangeul" => Key::Hangeul,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "hangul" => Key::Hangul,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "hanja" => Key::Hanja,
        "help" => Key::Help,
        "home" => Key::Home,
        #[cfg(target_os = "windows")]
        "ico00" => Key::Ico00,
        #[cfg(target_os = "windows")]
        "icoclear" => Key::IcoClear,
        #[cfg(target_os = "windows")]
        "icohelp" => Key::IcoHelp,
        #[cfg(target_os = "macos")]
        "illuminationdown" => Key::IlluminationDown,
        #[cfg(target_os = "macos")]
        "illuminationup" => Key::IlluminationUp,
        #[cfg(target_os = "macos")]
        "illuminationtoggle" => Key::IlluminationToggle,
        #[cfg(target_os = "windows")]
        "imeoff" => Key::IMEOff,
        #[cfg(target_os = "windows")]
        "imeon" => Key::IMEOn,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "insert" => Key::Insert,
        #[cfg(target_os = "windows")]
        "junja" => Key::Junja,
        #[cfg(target_os = "windows")]
        "kana" => Key::Kana,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "kanji" => Key::Kanji,
        #[cfg(target_os = "windows")]
        "launchapp1" => Key::LaunchApp1,
        #[cfg(target_os = "windows")]
        "launchapp2" => Key::LaunchApp2,
        #[cfg(target_os = "windows")]
        "launchmail" => Key::LaunchMail,
        #[cfg(target_os = "windows")]
        "launchmediaselect" => Key::LaunchMediaSelect,
        #[cfg(target_os = "macos")]
        "launchpad" => Key::Launchpad,
        #[cfg(target_os = "macos")]
        "launchpanel" => Key::LaunchPanel,
        #[cfg(target_os = "windows")]
        "lbutton" => Key::LButton,
        "lcontrol" => Key::LControl,
        "leftarrow" => Key::LeftArrow,
        #[cfg(all(unix, not(target_os = "macos")))]
        "linefeed" => Key::Linefeed,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "lmenu" => Key::LMenu,
        "lshift" => Key::LShift,
        #[cfg(target_os = "windows")]
        "lwin" => Key::LWin,
        #[cfg(target_os = "windows")]
        "mbutton" => Key::MButton,
        #[cfg(target_os = "macos")]
        "mediafast" => Key::MediaFast,
        "medianexttrack" => Key::MediaNextTrack,
        "mediaplaypause" => Key::MediaPlayPause,
        "mediaprevtrack" => Key::MediaPrevTrack,
        #[cfg(target_os = "macos")]
        "mediarewind" => Key::MediaRewind,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "mediastop" => Key::MediaStop,
        "meta" => Key::Meta,
        #[cfg(target_os = "macos")]
        "missioncontrol" => Key::MissionControl,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "modechange" => Key::ModeChange,
        "multiply" => Key::Multiply,
        #[cfg(target_os = "windows")]
        "navigationaccept" => Key::NavigationAccept,
        #[cfg(target_os = "windows")]
        "navigationcancel" => Key::NavigationCancel,
        #[cfg(target_os = "windows")]
        "navigationdown" => Key::NavigationDown,
        #[cfg(target_os = "windows")]
        "navigationleft" => Key::NavigationLeft,
        #[cfg(target_os = "windows")]
        "navigationmenu" => Key::NavigationMenu,
        #[cfg(target_os = "windows")]
        "navigationright" => Key::NavigationRight,
        #[cfg(target_os = "windows")]
        "navigationup" => Key::NavigationUp,
        #[cfg(target_os = "windows")]
        "navigationview" => Key::NavigationView,
        #[cfg(target_os = "windows")]
        "noname" => Key::NoName,
        #[cfg(target_os = "windows")]
        "nonconvert" => Key::NonConvert,
        #[cfg(target_os = "windows")]
        "none" => Key::None,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "numlock" => Key::Numlock,
        "numpad0" => Key::Numpad0,
        "numpad1" => Key::Numpad1,
        "numpad2" => Key::Numpad2,
        "numpad3" => Key::Numpad3,
        "numpad4" => Key::Numpad4,
        "numpad5" => Key::Numpad5,
        "numpad6" => Key::Numpad6,
        "numpad7" => Key::Numpad7,
        "numpad8" => Key::Numpad8,
        "numpad9" => Key::Numpad9,
        #[cfg(target_os = "windows")]
        "oem1" => Key::OEM1,
        #[cfg(target_os = "windows")]
        "oem102" => Key::OEM102,
        #[cfg(target_os = "windows")]
        "oem2" => Key::OEM2,
        #[cfg(target_os = "windows")]
        "oem3" => Key::OEM3,
        #[cfg(target_os = "windows")]
        "oem4" => Key::OEM4,
        #[cfg(target_os = "windows")]
        "oem5" => Key::OEM5,
        #[cfg(target_os = "windows")]
        "oem6" => Key::OEM6,
        #[cfg(target_os = "windows")]
        "oem7" => Key::OEM7,
        #[cfg(target_os = "windows")]
        "oem8" => Key::OEM8,
        #[cfg(target_os = "windows")]
        "oemattn" => Key::OEMAttn,
        #[cfg(target_os = "windows")]
        "oemauto" => Key::OEMAuto,
        #[cfg(target_os = "windows")]
        "oemax" => Key::OEMAx,
        #[cfg(target_os = "windows")]
        "oembacktab" => Key::OEMBacktab,
        #[cfg(target_os = "windows")]
        "oemclear" => Key::OEMClear,
        #[cfg(target_os = "windows")]
        "oemcomma" => Key::OEMComma,
        #[cfg(target_os = "windows")]
        "oemcopy" => Key::OEMCopy,
        #[cfg(target_os = "windows")]
        "oemcusel" => Key::OEMCusel,
        #[cfg(target_os = "windows")]
        "oemenlw" => Key::OEMEnlw,
        #[cfg(target_os = "windows")]
        "oemfinish" => Key::OEMFinish,
        #[cfg(target_os = "windows")]
        "oemfjjisho" => Key::OEMFJJisho,
        #[cfg(target_os = "windows")]
        "oemfjloya" => Key::OEMFJLoya,
        #[cfg(target_os = "windows")]
        "oemfjmasshou" => Key::OEMFJMasshou,
        #[cfg(target_os = "windows")]
        "oemfjroya" => Key::OEMFJRoya,
        #[cfg(target_os = "windows")]
        "oemfjtouroku" => Key::OEMFJTouroku,
        #[cfg(target_os = "windows")]
        "oemjump" => Key::OEMJump,
        #[cfg(target_os = "windows")]
        "oemminus" => Key::OEMMinus,
        #[cfg(target_os = "windows")]
        "oemnecequal" => Key::OEMNECEqual,
        #[cfg(target_os = "windows")]
        "oempa1" => Key::OEMPA1,
        #[cfg(target_os = "windows")]
        "oempa2" => Key::OEMPA2,
        #[cfg(target_os = "windows")]
        "oempa3" => Key::OEMPA3,
        #[cfg(target_os = "windows")]
        "oemperiod" => Key::OEMPeriod,
        #[cfg(target_os = "windows")]
        "oemplus" => Key::OEMPlus,
        #[cfg(target_os = "windows")]
        "oemreset" => Key::OEMReset,
        #[cfg(target_os = "windows")]
        "oemwsctrl" => Key::OEMWsctrl,
        "option" => Key::Option,
        #[cfg(target_os = "windows")]
        "pa1" => Key::PA1,
        #[cfg(target_os = "windows")]
        "packet" => Key::Packet,
        "pagedown" => Key::PageDown,
        "pageup" => Key::PageUp,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "pause" => Key::Pause,
        #[cfg(target_os = "windows")]
        "play" => Key::Play,
        #[cfg(target_os = "macos")]
        "power" => Key::Power,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "printscr" => Key::PrintScr,
        #[cfg(target_os = "windows")]
        "processkey" => Key::Processkey,
        #[cfg(target_os = "windows")]
        "rbutton" => Key::RButton,
        #[cfg(target_os = "macos")]
        "rcommand" => Key::RCommand,
        "rcontrol" => Key::RControl,
        #[cfg(all(unix, not(target_os = "macos")))]
        "redo" => Key::Redo,
        "return" => Key::Return,
        "rightarrow" => Key::RightArrow,
        #[cfg(target_os = "windows")]
        "rmenu" => Key::RMenu,
        #[cfg(target_os = "macos")]
        "roption" => Key::ROption,
        "rshift" => Key::RShift,
        #[cfg(target_os = "windows")]
        "rwin" => Key::RWin,
        #[cfg(target_os = "windows")]
        "scroll" => Key::Scroll,
        #[cfg(all(unix, not(target_os = "macos")))]
        "scrolllock" => Key::ScrollLock,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "select" => Key::Select,
        #[cfg(all(unix, not(target_os = "macos")))]
        "scriptswitch" => Key::ScriptSwitch,
        #[cfg(target_os = "windows")]
        "separator" => Key::Separator,
        "shift" => Key::Shift,
        #[cfg(all(unix, not(target_os = "macos")))]
        "shiftlock" => Key::ShiftLock,
        #[cfg(target_os = "windows")]
        "sleep" => Key::Sleep,
        "space" => Key::Space,
        "subtract" => Key::Subtract,
        #[cfg(all(unix, not(target_os = "macos")))]
        "sysreq" => Key::SysReq,
        "tab" => Key::Tab,
        #[cfg(all(unix, not(target_os = "macos")))]
        "undo" => Key::Undo,
        "uparrow" => Key::UpArrow,
        #[cfg(target_os = "macos")]
        "vidmirror" => Key::VidMirror,
        "volumedown" => Key::VolumeDown,
        "volumemute" => Key::VolumeMute,
        "volumeup" => Key::VolumeUp,
        #[cfg(all(unix, not(target_os = "macos")))]
        "micmute" => Key::MicMute,
        #[cfg(target_os = "windows")]
        "xbutton1" => Key::XButton1,
        #[cfg(target_os = "windows")]
        "xbutton2" => Key::XButton2,
        #[cfg(target_os = "windows")]
        "zoom" => Key::Zoom,
        "enter" => Key::Return,
        "esc" => Key::Escape,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "ctrl" => Key::Control,
        "cmd" => Key::Meta,
        "win" => Key::Meta,
        "super" => Key::Meta,
        "pgup" => Key::PageUp,
        "pgdn" => Key::PageDown,
        "del" => Key::Delete,
        "print" => Key::PrintScr,
        "snapshot" => Key::PrintScr,
        s if s.chars().count() == 1 => Key::Unicode(s.chars().next().unwrap()),
        _ => return Err(mlua::Error::external(format!("unsupported key: {}", key_str))),
      };
      
      let parsed_dir = match direction.to_lowercase().as_str() {
        "press" | "p" => Direction::Press,
        "release" | "r" => Direction::Release,
        "click" | "c" => Direction::Click,
        _ => return Err(mlua::Error::external(format!("unsupported direction: {}", direction))),
      };
      
      let mut enigo_guard = enigo_clone.lock().unwrap();
      enigo_guard.0.key(parsed_key, parsed_dir).map_err(|e| e.into_lua_err())?;
      Ok(())
    },
  )?;

  static DO_MAIN_LOOP: AtomicBool = AtomicBool::new(false);

  let main_loop = lua.create_function(move |_, ()| -> Result<(), mlua::Error> {
    DO_MAIN_LOOP.swap(true, Ordering::Relaxed);
    Ok(())
  })?;

  let globals = lua.globals();
  globals.set("device_open", &device_open)?;
  globals.set("device_is_connected", &device_is_connected)?;
  globals.set("msgbox", &msgbox)?;
  globals.set("sleep_ms", &sleep_ms)?;
  globals.set("register_hotkey", &register_hotkey)?;
  globals.set("register_hotplug", &register_hotplug)?;
  globals.set("register_screen_edge", &register_screen_edge)?;
  globals.set("main_loop", &main_loop)?;
  globals.set("host_os", std::env::consts::OS)?;
  globals.set("host_arch", std::env::consts::ARCH)?;
  globals.set("host_family", std::env::consts::FAMILY)?;
  globals.set("autorun", autorun)?;
  globals.set("register_interval", &register_interval)?;

  globals.set("unregister_interval", &unregister_interval)?;
  globals.set("move_mouse", &move_mouse)?;
  globals.set("screen_size", &screen_size)?;
  globals.set("key", &key)?;

  let cmd_path = std::path::Path::new(&args.cmd);
  if cmd_path.is_file() {
    let source = std::fs::read_to_string(cmd_path)
      .map_err(|e| mlua::Error::RuntimeError(format!("could not read '{}': {}", args.cmd, e)))?;
    lua.load(&source).set_name(&args.cmd).exec()?;
  } else {
    lua.load(&args.cmd).exec()?;
  }

  if DO_MAIN_LOOP.load(Ordering::Relaxed) {
    event!(Level::INFO, "starting main loop");
    let hotkeys_clone = hotkeys.clone();
    let hotplug_clone = hotplug.clone();
    let devices_clone = devices.clone();
    let interval_callbacks_clone = interval_callbacks.clone();
    let screen_edge_clone = screen_edge.clone();
    let rx = hotplug_rx.clone();
    let mut last_screen_edge: Option<&'static str> = None;
    let mut last_edge_check = std::time::Instant::now();
    let mut cached_displays: Vec<DisplayInfo> = Vec::new();
    let mut last_displays_refresh: Option<std::time::Instant> = None;
    event_loop.run(move |_, _, control_flow| {
      *control_flow = ControlFlow::Poll;
      // *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

      if screen_edge_clone.lock().unwrap().is_some()
        && last_edge_check.elapsed() >= Duration::from_millis(50)
      {
        last_edge_check = std::time::Instant::now();

        let needs_refresh =
          last_displays_refresh.map_or(true, |t| t.elapsed() >= Duration::from_secs(5));
        if needs_refresh {
          if let Ok(d) = DisplayInfo::all() {
            cached_displays = d;
          }
          last_displays_refresh = Some(std::time::Instant::now());
        }

        let edge = match Mouse::get_mouse_position() {
          Mouse::Position {
            x,
            y,
          } => find_screen_edge(&cached_displays, x, y),
          Mouse::Error => None,
        };
        if edge != last_screen_edge {
          if let Some(e) = edge
            && let Some(cb) = screen_edge_clone.lock().unwrap().as_ref()
          {
            if let Err(err) = cb.call::<()>((e,)) {
              event!(Level::ERROR, "screen_edge callback: {}", err);
            }
          }
          last_screen_edge = edge;
        }
      }

      if let Ok(mut ic) = interval_callbacks_clone.lock() {
        for (_k, v) in &mut *ic {
          if std::time::Instant::now() >= v.2 {
            if let Err(err) = v.0.call::<()>(()) {
              event!(Level::ERROR, "interval callback: {}", err);
            }
            let interval = rand::random_range(v.1.0..=v.1.1);
            v.2 = std::time::Instant::now() + std::time::Duration::from_millis(interval);
          }
        }
      }

      if let Ok(hk_event) = global_hotkey_channel.try_recv() {
        let hk = hotkeys_clone.lock().unwrap();
        for (hk, callback) in hk.iter() {
          if hk.id() == hk_event.id() && hk_event.state == HotKeyState::Released {
            if let Err(err) = callback.call::<()>((hk.to_string(),)) {
              event!(Level::ERROR, "hotkey callback: {}", err);
            }
          }
        }
      }

      if let Ok(hotplug_event) = rx.try_recv() {
        let hp = hotplug_clone.lock().unwrap();
        if let HotplugEvent::Connected(_) = hotplug_event {
          // When we connect again, make sure all our modifier keys are not pressed down.
          let device_state = device_query::DeviceState::new();
          let keys = device_state.get_keys();
          if keys.contains(&device_query::Keycode::LAlt) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::Alt)).unwrap();
          }
          if keys.contains(&device_query::Keycode::RAlt) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::AltGr)).unwrap();
          }
          if keys.contains(&device_query::Keycode::LControl) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ControlLeft)).unwrap();
          }
          if keys.contains(&device_query::Keycode::RControl) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ControlRight)).unwrap();
          }
          if keys.contains(&device_query::Keycode::LShift) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ShiftLeft)).unwrap();
          }
          if keys.contains(&device_query::Keycode::RShift) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ShiftRight)).unwrap();
          }
          if keys.contains(&device_query::Keycode::LMeta) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::MetaLeft)).unwrap();
          }
          if keys.contains(&device_query::Keycode::RMeta) {
            rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::MetaRight)).unwrap();
          }
        }

        if let Some(cb) = hp.clone() {
          match hotplug_event {
            HotplugEvent::Connected(d) => {
              if let Err(err) = cb.call::<()>(("connected", d.vendor_id(), d.product_id())) {
                event!(Level::ERROR, "hotplug connected callback: {}", err);
              }
              let mut devices = devices_clone.lock().unwrap();
              devices.insert(d.id(), d);
            },
            HotplugEvent::Disconnected(id) => {
              let mut devices = devices_clone.lock().unwrap();
              if let Some(d) = devices.get(&id) {
                if let Err(err) = cb.call::<()>(("disconnected", d.vendor_id(), d.product_id())) {
                  event!(Level::ERROR, "hotplug disconnected callback: {}", err);
                }
                devices.remove(&id);
              }
            },
          };
        };
      }
    });
  }

  Ok(())
}

fn setup_logging() -> Result<(), Box<StdError>> {
  let project_dirs = ProjectDirs::from("com", "kdar", env!("CARGO_CRATE_NAME"))
    .ok_or("could not find project dir")?;
  let config_dir = project_dirs.data_local_dir().to_path_buf();

  let file_appender = tracing_appender::rolling::never(&config_dir, "app.log");

  tracing_subscriber::registry()
    .with(
      tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_file(true)
        .with_line_number(true)
        .with_ansi(false),
    )
    .with(tracing_subscriber::fmt::layer().pretty())
    .with(
      EnvFilter::builder()
        .with_default_directive(LevelFilter::DEBUG.into())
        .with_env_var(EnvFilter::DEFAULT_ENV)
        .from_env_lossy(),
    )
    .init();

  Ok(())
}

fn main() {
  if let Err(err) = setup_logging() {
    if std::io::stdout().is_terminal() {
      event!(
        Level::ERROR,
        error = err.to_string(),
        "Could not setup logging"
      );
    } else {
      let dialog = MessageDialog::new()
        .set_title("Error")
        .set_description(format!("Could not setup logging: {}", err))
        .set_buttons(MessageButtons::Ok)
        .set_level(MessageLevel::Error);
      dialog.show();
    }
  }

  if let Err(err) = run() {
    let dialog = MessageDialog::new()
      .set_title("Error")
      .set_description(format!("Error: {}", err))
      .set_buttons(MessageButtons::Ok)
      .set_level(MessageLevel::Error);
    dialog.show();
    event!(Level::ERROR, error = err.to_string());
  }
}
