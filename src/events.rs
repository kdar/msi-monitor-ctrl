use std::sync::atomic::Ordering;
use std::time::Duration;

use device_query::DeviceQuery;
use display_info::DisplayInfo;
use global_hotkey::GlobalHotKeyEvent;
use global_hotkey::HotKeyState;
use mlua::Lua;
use mouse_position::mouse_position::Mouse as MousePos;
use nusb::hotplug::HotplugEvent;
use tao::event_loop::ControlFlow;
use tao::event_loop::EventLoop;
use tracing::Level;
use tracing::event;

use crate::find_screen_edge;
use crate::state::AppState;

pub fn run_loop(
  event_loop: EventLoop<()>,
  _lua: Lua,
  state: AppState,
  hotplug_rx: crossbeam_channel::Receiver<HotplugEvent>,
  global_hotkey_channel: crossbeam_channel::Receiver<GlobalHotKeyEvent>,
) -> Result<(), mlua::Error> {
  if crate::state::DO_MAIN_LOOP.load(Ordering::Relaxed) {
    event!(Level::INFO, "starting main loop");
    let hotkeys_clone = state.hotkeys.clone();
    let hotplug_clone = state.hotplug.clone();
    let devices_clone = state.devices.clone();
    let interval_callbacks_clone = state.interval_callbacks.clone();
    let screen_edge_clone = state.screen_edge.clone();
    let rx = hotplug_rx;
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
          last_displays_refresh.is_none_or(|t| t.elapsed() >= Duration::from_secs(5));
        if needs_refresh {
          if let Ok(d) = DisplayInfo::all() {
            cached_displays = d;
          }
          last_displays_refresh = Some(std::time::Instant::now());
        }

        let edge = match MousePos::get_mouse_position() {
          MousePos::Position {
            x,
            y,
          } => find_screen_edge(&cached_displays, x, y),
          MousePos::Error => None,
        };
        if edge != last_screen_edge {
          if let Some(e) = edge
            && let Some(cb) = screen_edge_clone.lock().unwrap().as_ref()
            && let Err(err) = cb.call::<()>((e,))
          {
            event!(Level::ERROR, "screen_edge callback: {}", err);
          }
          last_screen_edge = edge;
        }
      }

      if let Ok(mut ic) = interval_callbacks_clone.lock() {
        for v in (*ic).values_mut() {
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
          if hk.id() == hk_event.id()
            && hk_event.state == HotKeyState::Released
            && let Err(err) = callback.call::<()>((hk.to_string(),))
          {
            event!(Level::ERROR, "hotkey callback: {}", err);
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
