#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use std::io::IsTerminal;

use clap::Parser;
use directories::ProjectDirs;
use display_info::DisplayInfo;
use errors::StdError;
use global_hotkey::GlobalHotKeyEvent;
use global_hotkey::GlobalHotKeyManager;
use mlua::Lua;
use rfd::MessageButtons;
use rfd::MessageDialog;
use rfd::MessageLevel;
use tao::event_loop::EventLoop;
use tracing::Level;
use tracing::event;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod api;
mod device;
mod errors;
mod events;
mod state;
use state::AppState;

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

  use std::collections::HashMap;
  use std::sync::Arc;
  use std::sync::Mutex;

    use enigo::Settings;
use enigo::Enigo;
use nusb::MaybeFuture;
  
  use crate::state::WrappedEnigo;
  use crate::state::WrappedHotKeyManager;

  let state = AppState {
    hotkeys: Arc::new(Mutex::new(HashMap::new())),
    hk_manager: Arc::new(Mutex::new(WrappedHotKeyManager(hotkeys_manager))),
    hotplug,
    screen_edge: Arc::new(Mutex::new(None)),
    interval_callbacks: Arc::new(Mutex::new(HashMap::new())),
    devices: Arc::new(Mutex::new(
      nusb::list_devices()
        .wait()
        .unwrap()
        .map(|d| (d.id(), d))
        .collect(),
    )),
    enigo: Arc::new(Mutex::new(WrappedEnigo(
      Enigo::new(&Settings::default()).unwrap(),
    ))),
  };

  api::register(&lua, state.clone()).unwrap();

  let cmd_path = std::path::Path::new(&args.cmd);
  if cmd_path.is_file() {
    let source = std::fs::read_to_string(cmd_path)
      .map_err(|e| mlua::Error::RuntimeError(format!("could not read '{}': {}", args.cmd, e)))?;
    lua.load(&source).set_name(&args.cmd).exec()?;
  } else {
    lua.load(&args.cmd).exec()?;
  }

  events::run_loop(
    event_loop,
    lua,
    state,
    hotplug_rx,
    global_hotkey_channel.clone(),
  )?;
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
