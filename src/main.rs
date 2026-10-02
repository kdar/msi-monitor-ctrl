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
        "num0" => Key::Num0,
        "num1" => Key::Num1,
        "num2" => Key::Num2,
        "num3" => Key::Num3,
        "num4" => Key::Num4,
        "num5" => Key::Num5,
        "num6" => Key::Num6,
        "num7" => Key::Num7,
        "num8" => Key::Num8,
        "num9" => Key::Num9,
        "a" => Key::A,
        "b" => Key::B,
        "c" => Key::C,
        "d" => Key::D,
        "e" => Key::E,
        "f" => Key::F,
        "g" => Key::G,
        "h" => Key::H,
        "i" => Key::I,
        "j" => Key::J,
        "k" => Key::K,
        "l" => Key::L,
        "m" => Key::M,
        "n" => Key::N,
        "o" => Key::O,
        "p" => Key::P,
        "q" => Key::Q,
        "r" => Key::R,
        "s" => Key::S,
        "t" => Key::T,
        "u" => Key::U,
        "v" => Key::V,
        "w" => Key::W,
        "x" => Key::X,
        "y" => Key::Y,
        "z" => Key::Z,
        "accept" => Key::Accept,
        "add" => Key::Add,
        "alt" => Key::Alt,
        "apps" => Key::Apps,
        "backspace" => Key::Backspace,
        "browserback" => Key::BrowserBack,
        "browserfavorites" => Key::BrowserFavorites,
        "browserforward" => Key::BrowserForward,
        "browserhome" => Key::BrowserHome,
        "browserrefresh" => Key::BrowserRefresh,
        "browsersearch" => Key::BrowserSearch,
        "browserstop" => Key::BrowserStop,
        "cancel" => Key::Cancel,
        "capslock" => Key::CapsLock,
        "control" => Key::Control,
        "convert" => Key::Convert,
        "decimal" => Key::Decimal,
        "delete" => Key::Delete,
        "divide" => Key::Divide,
        "downarrow" => Key::DownArrow,
        "end" => Key::End,
        "escape" => Key::Escape,
        "execute" => Key::Execute,
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
        "final" => Key::Final,
        "gamepada" => Key::GamepadA,
        "gamepadb" => Key::GamepadB,
        "gamepaddpaddown" => Key::GamepadDPadDown,
        "gamepaddpadleft" => Key::GamepadDPadLeft,
        "gamepaddpadright" => Key::GamepadDPadRight,
        "gamepaddpadup" => Key::GamepadDPadUp,
        "gamepadleftshoulder" => Key::GamepadLeftShoulder,
        "gamepadleftthumbstickbutton" => Key::GamepadLeftThumbstickButton,
        "gamepadleftthumbstickdown" => Key::GamepadLeftThumbstickDown,
        "gamepadleftthumbstickleft" => Key::GamepadLeftThumbstickLeft,
        "gamepadleftthumbstickright" => Key::GamepadLeftThumbstickRight,
        "gamepadleftthumbstickup" => Key::GamepadLeftThumbstickUp,
        "gamepadlefttrigger" => Key::GamepadLeftTrigger,
        "gamepadmenu" => Key::GamepadMenu,
        "gamepadrightshoulder" => Key::GamepadRightShoulder,
        "gamepadrightthumbstickbutton" => Key::GamepadRightThumbstickButton,
        "gamepadrightthumbstickdown" => Key::GamepadRightThumbstickDown,
        "gamepadrightthumbstickleft" => Key::GamepadRightThumbstickLeft,
        "gamepadrightthumbstickright" => Key::GamepadRightThumbstickRight,
        "gamepadrightthumbstickup" => Key::GamepadRightThumbstickUp,
        "gamepadrighttrigger" => Key::GamepadRightTrigger,
        "gamepadview" => Key::GamepadView,
        "gamepadx" => Key::GamepadX,
        "gamepady" => Key::GamepadY,
        "hangeul" => Key::Hangeul,
        "help" => Key::Help,
        "home" => Key::Home,
        "ico00" => Key::Ico00,
        "icoclear" => Key::IcoClear,
        "icohelp" => Key::IcoHelp,
        "imeoff" => Key::IMEOff,
        "imeon" => Key::IMEOn,
        "insert" => Key::Insert,
        "junja" => Key::Junja,
        "launchapp1" => Key::LaunchApp1,
        "launchapp2" => Key::LaunchApp2,
        "launchmail" => Key::LaunchMail,
        "launchmediaselect" => Key::LaunchMediaSelect,
        "lbutton" => Key::LButton,
        "lcontrol" => Key::LControl,
        "leftarrow" => Key::LeftArrow,
        "lmenu" => Key::LMenu,
        "lshift" => Key::LShift,
        "lwin" => Key::LWin,
        "mbutton" => Key::MButton,
        "medianexttrack" => Key::MediaNextTrack,
        "mediaplaypause" => Key::MediaPlayPause,
        "mediaprevtrack" => Key::MediaPrevTrack,
        "mediastop" => Key::MediaStop,
        "meta" => Key::Meta,
        "modechange" => Key::ModeChange,
        "multiply" => Key::Multiply,
        "navigationaccept" => Key::NavigationAccept,
        "navigationcancel" => Key::NavigationCancel,
        "navigationdown" => Key::NavigationDown,
        "navigationleft" => Key::NavigationLeft,
        "navigationmenu" => Key::NavigationMenu,
        "navigationright" => Key::NavigationRight,
        "navigationup" => Key::NavigationUp,
        "navigationview" => Key::NavigationView,
        "none" => Key::None,
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
        "oem1" => Key::OEM1,
        "oem2" => Key::OEM2,
        "oem3" => Key::OEM3,
        "oem4" => Key::OEM4,
        "oem5" => Key::OEM5,
        "oem6" => Key::OEM6,
        "oem7" => Key::OEM7,
        "oem8" => Key::OEM8,
        "oemax" => Key::OEMAx,
        "oemcomma" => Key::OEMComma,
        "oemfjjisho" => Key::OEMFJJisho,
        "oemfjloya" => Key::OEMFJLoya,
        "oemfjmasshou" => Key::OEMFJMasshou,
        "oemfjroya" => Key::OEMFJRoya,
        "oemfjtouroku" => Key::OEMFJTouroku,
        "oemminus" => Key::OEMMinus,
        "oemnecequal" => Key::OEMNECEqual,
        "oemperiod" => Key::OEMPeriod,
        "oemplus" => Key::OEMPlus,
        "option" => Key::Option,
        "pagedown" => Key::PageDown,
        "pageup" => Key::PageUp,
        "pause" => Key::Pause,
        "print" => Key::PrintScr,
        "printscr" => Key::PrintScr,
        "rbutton" => Key::RButton,
        "rcontrol" => Key::RControl,
        "return" => Key::Return,
        "rightarrow" => Key::RightArrow,
        "rmenu" => Key::RMenu,
        "rshift" => Key::RShift,
        "rwin" => Key::RWin,
        "scroll" => Key::Scroll,
        "select" => Key::Select,
        "separator" => Key::Separator,
        "shift" => Key::Shift,
        "sleep" => Key::Sleep,
        "snapshot" => Key::PrintScr,
        "space" => Key::Space,
        "subtract" => Key::Subtract,
        "super" => Key::Meta,
        "tab" => Key::Tab,
        "uparrow" => Key::UpArrow,
        "volumedown" => Key::VolumeDown,
        "volumemute" => Key::VolumeMute,
        "volumeup" => Key::VolumeUp,
        "windows" => Key::Meta,
        "xbutton1" => Key::XButton1,
        "xbutton2" => Key::XButton2,
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
