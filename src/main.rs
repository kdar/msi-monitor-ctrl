use std::{
  collections::HashMap,
  str::FromStr,
  sync::{Arc, Mutex},
  thread,
  time::{Duration, Instant},
};

use clap::Parser;
use errors::StdError;
use futures_lite::stream;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use mlua::{Function, Lua};
use nusb::hotplug::HotplugEvent;
use rfd::{MessageButtons, MessageDialog, MessageLevel};
use tao::event_loop::{ControlFlow, EventLoop};

mod device;
mod errors;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  #[arg(short, long)]
  cmd: String,
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

fn main() -> Result<(), Box<StdError>> {
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

  use std::str::FromStr;

  use simple_mdns::{InstanceInformation, sync_discovery::ServiceDiscovery};

  thread::spawn(|| {
    let mut discovery = ServiceDiscovery::new(
      InstanceInformation::new("a".into()).with_socket_address(
        "192.168.1.130:8090"
          .parse()
          .expect("Invalid socket address"),
      ),
      "_msi_monitor_ctrl._tcp.local",
      60,
    )
    .expect("Failed to start service discovery");

    loop {
      for x in discovery.get_known_services() {
        println!("{:?}", x);
      }

      thread::sleep(Duration::from_secs(5));
    }
  });

  let args = Args::parse();

  let lua = Lua::new();

  let hotkeys_manager = GlobalHotKeyManager::new().unwrap();
  let global_hotkey_channel = GlobalHotKeyEvent::receiver();
  let hotplug = Arc::new(Mutex::new(None));

  let (tx, rx) = crossbeam_channel::unbounded();
  std::thread::spawn(move || {
    for event in stream::block_on(nusb::watch_devices().unwrap()) {
      tx.send(event).unwrap();
    }
  });

  let open = lua.create_function(
    |_, (vendor_id, product_id): (u16, u16)| -> Result<device::MSIDevice, mlua::Error> {
      let dev = device::MSIDevice::open(vendor_id, product_id)
        .map_err(mlua::ExternalError::into_lua_err)?;
      Ok(dev)
    },
  )?;

  let sleep_ms = lua.create_function(|_, duration: u64| -> Result<(), mlua::Error> {
    thread::sleep(Duration::from_millis(duration));
    Ok(())
  })?;

  let hotkeys = Arc::new(Mutex::new(HashMap::new()));

  let hotkeys_clone = hotkeys.clone();
  let register_hotkey = lua.create_function(
    move |_, (keybind, callback): (String, Function)| -> Result<(), mlua::Error> {
      let hotkey = HotKey::from_str(&keybind).map_err(mlua::ExternalError::into_lua_err)?;
      hotkeys_manager
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
  let main_loop = lua.create_function(move |_, ()| -> Result<(), mlua::Error> {
    let event_loop = EventLoop::new();
    let hotkeys_clone = hotkeys.clone();
    let hotplug_clone = hotplug.clone();
    let devices_clone = devices.clone();
    let rx = rx.clone();
    event_loop.run(move |_, _, control_flow| {
      *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

      if let Ok(hk_event) = global_hotkey_channel.try_recv() {
        let hk = hotkeys_clone.lock().unwrap();
        for (hk, callback) in hk.iter() {
          if hk.id() == hk_event.id() && hk_event.state == HotKeyState::Released {
            // This releases the modifer key which could cause it to get stuck if
            // we were to switch KVM.
            if hk.mods.contains(global_hotkey::hotkey::Modifiers::ALT) {
              rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::Alt)).unwrap();
            }
            if hk.mods.contains(global_hotkey::hotkey::Modifiers::CONTROL) {
              rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ControlLeft)).unwrap();
              rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ControlRight)).unwrap();
            }
            if hk.mods.contains(
              global_hotkey::hotkey::Modifiers::META | global_hotkey::hotkey::Modifiers::SUPER,
            ) {
              rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::MetaLeft)).unwrap();
              rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::MetaRight)).unwrap();
            }
            if hk.mods.contains(global_hotkey::hotkey::Modifiers::SHIFT) {
              rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ShiftLeft)).unwrap();
              rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ShiftRight)).unwrap();
            }

            callback.call::<()>((hk.to_string(),)).unwrap();
          }
        }
      }

      if let Ok(event) = rx.try_recv() {
        let hp = hotplug_clone.lock().unwrap();
        if let Some(cb) = hp.clone() {
          match event {
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
  })?;

  let globals = lua.globals();
  globals.set("open", &open)?;
  globals.set("msgbox", &msgbox)?;
  globals.set("sleep_ms", &sleep_ms)?;
  globals.set("register_hotkey", &register_hotkey)?;
  globals.set("register_hotplug", &register_hotplug)?;
  globals.set("main_loop", &main_loop)?;
  globals.set("host_os", std::env::consts::OS)?;
  globals.set("host_arch", std::env::consts::ARCH)?;
  globals.set("host_family", std::env::consts::FAMILY)?;

  lua.load(args.cmd).exec()?;

  Ok(())
}
