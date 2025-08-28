#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use std::{
  collections::HashMap,
  io::IsTerminal,
  str::FromStr,
  sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
  },
  thread,
  time::{Duration, Instant},
};

use clap::Parser;
use device_query::DeviceQuery;
use directories::ProjectDirs;
use errors::StdError;
use futures_lite::stream;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use mlua::{ExternalError, Function, Lua};
use nusb::hotplug::HotplugEvent;
use rfd::{MessageButtons, MessageDialog, MessageLevel};
use rustautogui::RustAutoGui;
use tao::event_loop::{ControlFlow, EventLoop};
use tracing::{Level, event, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod device;
mod errors;

static INTERVAL_COUNTER: AtomicUsize = AtomicUsize::new(1);

fn get_interval_id() -> usize {
  INTERVAL_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// We wrap GlobalHotKeyManager so we can send it across threads. This is
// safe to do for specific windows pointers like HWND since
// it is unique globally.
struct WrappedHotKeyManager(GlobalHotKeyManager);

#[cfg(target_os = "windows")]
unsafe impl Send for WrappedHotKeyManager {}
#[cfg(target_os = "windows")]
unsafe impl Sync for WrappedHotKeyManager {}

// We wrap RustAutoGui so we can send it across threads. This is
// safe to do for specific windows pointers like HWND since
// it is unique globally.
struct WrappedRustAutoGui(rustautogui::RustAutoGui);

#[cfg(target_os = "windows")]
unsafe impl Send for WrappedRustAutoGui {}
#[cfg(target_os = "windows")]
unsafe impl Sync for WrappedRustAutoGui {}

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
    for event in stream::block_on(nusb::watch_devices().unwrap()) {
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
    nusb::list_devices().unwrap().map(|d| (d.id(), d)).collect(),
  ));

  let autorun = lua.create_function(
    |_, (app_path, args): (Option<String>, Option<Vec<String>>)| -> Result<(), mlua::Error> {
      let mut autolaunch = auto_launch::AutoLaunchBuilder::new();

      autolaunch
        .set_app_name(env!("CARGO_CRATE_NAME"))
        .set_use_launch_agent(true);

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

  let rustautogui = Arc::new(Mutex::new(WrappedRustAutoGui(
    RustAutoGui::new(false).map_err(|e| e.into_lua_err())?,
  )));
  let rustautogui_clone = rustautogui.clone();
  let screen_size = lua.create_function(move |_, ()| -> Result<(i32, i32), mlua::Error> {
    let mut rag = rustautogui_clone.lock().unwrap();
    Ok(rag.0.get_screen_size())
  })?;
  let rustautogui_clone = rustautogui.clone();
  let move_mouse = lua.create_function(
    move |_, (x, y, moving_time, mode): (i64, i64, f32, String)| -> Result<(), mlua::Error> {
      // Anything > 10.0 is VERY slow and can lock your computer.
      let moving_time = if moving_time > 10.0 {
        10.0
      } else {
        moving_time
      };

      let rag = rustautogui_clone.lock().unwrap();
      match mode.as_str() {
        "rel" => {
          rag
            .0
            .move_mouse(
              i32::try_from(x).map_err(|e| e.into_lua_err())?,
              i32::try_from(y).map_err(|e| e.into_lua_err())?,
              moving_time,
            )
            .map_err(|e| e.into_lua_err())?;
        },
        "abs" => {
          rag
            .0
            .move_mouse_to_pos(
              u32::try_from(x).map_err(|e| e.into_lua_err())?,
              u32::try_from(y).map_err(|e| e.into_lua_err())?,
              moving_time,
            )
            .map_err(|e| e.into_lua_err())?;
        },
        _ => {
          return Err(mlua::Error::external(format!(
            "unknown move_mouse mode: {}",
            mode
          )));
        },
      };
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
  globals.set("main_loop", &main_loop)?;
  globals.set("host_os", std::env::consts::OS)?;
  globals.set("host_arch", std::env::consts::ARCH)?;
  globals.set("host_family", std::env::consts::FAMILY)?;
  globals.set("autorun", autorun)?;
  globals.set("register_interval", &register_interval)?;
  globals.set("unregister_interval", &unregister_interval)?;
  globals.set("move_mouse", &move_mouse)?;
  globals.set("screen_size", &screen_size)?;

  lua.load(args.cmd).exec()?;

  if DO_MAIN_LOOP.load(Ordering::Relaxed) {
    event!(Level::INFO, "starting main loop");
    let hotkeys_clone = hotkeys.clone();
    let hotplug_clone = hotplug.clone();
    let devices_clone = devices.clone();
    let interval_callbacks_clone = interval_callbacks.clone();
    let rx = hotplug_rx.clone();
    event_loop.run(move |_, _, control_flow| {
      *control_flow = ControlFlow::Poll;
      // *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

      if let Ok(mut ic) = interval_callbacks_clone.lock() {
        for (_k, v) in &mut *ic {
          if std::time::Instant::now() >= v.2 {
            v.0.call::<()>(()).unwrap();
            let interval = rand::random_range(v.1.0..=v.1.1);
            v.2 = std::time::Instant::now() + std::time::Duration::from_millis(interval);
          }
        }
      }

      if let Ok(hk_event) = global_hotkey_channel.try_recv() {
        let hk = hotkeys_clone.lock().unwrap();
        for (hk, callback) in hk.iter() {
          if hk.id() == hk_event.id() && hk_event.state == HotKeyState::Released {
            callback.call::<()>((hk.to_string(),)).unwrap();
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
              cb.call::<()>(("connected", d.vendor_id(), d.product_id()))
                .unwrap();
              let mut devices = devices_clone.lock().unwrap();
              devices.insert(d.id(), d);
            },
            HotplugEvent::Disconnected(id) => {
              let mut devices = devices_clone.lock().unwrap();
              if let Some(d) = devices.get(&id) {
                cb.call::<()>(("disconnected", d.vendor_id(), d.product_id()))
                  .unwrap();
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
