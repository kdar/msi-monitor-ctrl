use std::{
  error::Error,
  sync::{Arc, Mutex},
  thread,
  time::{Duration, Instant},
};

use futures_lite::stream;
use global_hotkey::{
  GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
  hotkey::{Code, HotKey, Modifiers},
};
use nusb::hotplug::HotplugEvent;
use rusb::{
  Device, DeviceDescriptor, DeviceHandle, Direction, GlobalContext, TransferType, UsbContext,
};
use tao::event_loop::{ControlFlow, EventLoop};

const VENDOR_ID: u16 = 0x1462;
const PRODUCT_ID: u16 = 0x3fa4;

#[derive(Debug)]
struct Endpoint {
  config: u8,
  iface: u8,
  setting: u8,
  address: u8,
}

pub fn packet(slice: &[u8]) -> [u8; 64] {
  let mut buffer = [0x00; 64];
  let n = std::cmp::min(buffer.len(), slice.len());
  buffer[0..n].copy_from_slice(&slice[0..n]);
  buffer
}

fn get_device() -> Result<Option<Device<GlobalContext>>, Box<dyn Error>> {
  for device in rusb::devices()?.iter() {
    let device_desc = device.device_descriptor()?;

    if device_desc.vendor_id() == VENDOR_ID && device_desc.product_id() == PRODUCT_ID {
      return Ok(Some(device));
    }
  }

  Ok(None)
}

fn switch_device(dev_handle: &mut DeviceHandle<GlobalContext>) -> Result<(), Box<dyn Error>> {
  let timeout = Duration::from_secs(2);

  let mut device = dev_handle.device();
  let device_desc = device.device_descriptor()?;

  let out_endpoint = find_endpoint(
    &mut device,
    &device_desc,
    Direction::Out,
    TransferType::Interrupt,
  )
  .unwrap();
  configure_endpoint(dev_handle, &out_endpoint)?;

  let in_endpoint = find_endpoint(
    &mut device,
    &device_desc,
    Direction::In,
    TransferType::Interrupt,
  )
  .unwrap();
  configure_endpoint(dev_handle, &in_endpoint)?;

  let buf = packet(&[
    0x01, 0x35, 0x62, 0x30, 0x30, 0x35, 0x30, 0x30, 0x30, 0x30, 0x33, 0x0d,
  ]);
  dev_handle.write_interrupt(out_endpoint.address, &buf, timeout)?;

  // let mut buf = [0x00; 64];
  // dev_handle.read_interrupt(in_endpoint.address, &mut buf, timeout)?;

  thread::sleep(Duration::from_secs(1));

  let buf = packet(&[
    0x01, 0x35, 0x62, 0x30, 0x30, 0x38, 0x3e, 0x30, 0x30, 0x30, 0x32, 0x0d,
  ]);
  dev_handle.write_interrupt(out_endpoint.address, &buf, timeout)?;

  // let mut buf = [0x00; 64];
  // dev_handle.read_interrupt(in_endpoint.address, &mut buf, timeout)?;

  Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
  let dev: Arc<Mutex<Option<DeviceHandle<GlobalContext>>>> = Arc::new(Mutex::new(None));

  if let Ok(Some(d)) = get_device() {
    let mut guard = dev.lock().unwrap();
    *guard = Some(d.open().unwrap());
  }

  let event_loop = EventLoop::new();

  let dev2 = dev.clone();
  std::thread::spawn(move || {
    let dev = dev2;
    let mut device_id = None;
    for event in stream::block_on(nusb::watch_devices().unwrap()) {
      match event {
        HotplugEvent::Connected(info) => {
          device_id = Some(info.id());
          if info.vendor_id() == VENDOR_ID && info.product_id() == PRODUCT_ID {
            let mut guard = dev.lock().unwrap();
            if let Ok(Some(d)) = get_device() {
              *guard = Some(d.open().unwrap());
            } else {
              *guard = None;
            }
          }
        },
        HotplugEvent::Disconnected(id) => {
          if device_id == Some(id) {
            let mut guard = dev.lock().unwrap();
            *guard = None;
          }
        },
      };
    }
  });

  let hotkeys_manager = GlobalHotKeyManager::new().unwrap();

  let hotkey = HotKey::new(
    Some(Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT),
    Code::ArrowRight,
  );

  hotkeys_manager.register(hotkey).unwrap();

  let global_hotkey_channel = GlobalHotKeyEvent::receiver();

  event_loop.run(move |_, _, control_flow| {
    // *control_flow = ControlFlow::Wait;
    *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

    if let Ok(hk_event) = global_hotkey_channel.try_recv() {
      if hotkey.id() == hk_event.id && hk_event.state == HotKeyState::Released {
        let mut guard = dev.lock().unwrap();
        match &mut *guard {
          Some(dev_handle) => {
            switch_device(dev_handle).unwrap();
          },
          None => {},
        }
      }
    }
  });
}

fn configure_endpoint<T: UsbContext>(
  handle: &mut DeviceHandle<T>,
  endpoint: &Endpoint,
) -> rusb::Result<()> {
  handle.set_active_configuration(endpoint.config)?;
  handle.claim_interface(endpoint.iface)?;
  handle.set_alternate_setting(endpoint.iface, endpoint.setting)?;
  Ok(())
}

fn find_endpoint<T: UsbContext>(
  device: &mut Device<T>,
  device_desc: &DeviceDescriptor,
  direction: Direction,
  transfer_type: TransferType,
) -> Option<Endpoint> {
  for n in 0..device_desc.num_configurations() {
    let config_desc = match device.config_descriptor(n) {
      Ok(c) => c,
      Err(_) => continue,
    };

    for interface in config_desc.interfaces() {
      for interface_desc in interface.descriptors() {
        for endpoint_desc in interface_desc.endpoint_descriptors() {
          if endpoint_desc.direction() == direction
            && endpoint_desc.transfer_type() == transfer_type
          {
            return Some(Endpoint {
              config: config_desc.number(),
              iface: interface_desc.interface_number(),
              setting: interface_desc.setting_number(),
              address: endpoint_desc.address(),
            });
          }
        }
      }
    }
  }

  None
}
