use std::{error::Error, thread, time::Duration};

use rusb::{
  Device, DeviceDescriptor, DeviceHandle, Direction, GlobalContext, TransferType, UsbContext,
};

use super::errors::StdError;

// This is the monitor index which I think increments
// when you have multiple of these monitors.
const INDEX: u8 = 0x1;
// This is the size of the generic return value when requesting data
// from the monitor. 3 bytes.
const RETURN_VALUE_NUM: usize = 3;
// This is the index of the end of a generic return from a command.
const ON_END_INDEX: usize = 10;

// usb.idVendor == 0x1462 && usb.idProduct == 0x3fa4

pub(crate) struct MSIDevice {
  device_handle: DeviceHandle<GlobalContext>,
  in_endpoint: Endpoint,
  out_endpoint: Endpoint,
}

impl MSIDevice {
  pub(crate) fn open(vendor_id: u16, product_id: u16) -> Result<Self, Box<StdError>> {
    let Some(mut device) = get_device(vendor_id, product_id)? else {
      return Err("unable to find device".into());
    };
    let mut device_handle = device.open()?;
    let device_desc = device.device_descriptor()?;

    let Some(out_endpoint) = find_endpoint(
      &mut device,
      &device_desc,
      Direction::Out,
      TransferType::Interrupt,
    ) else {
      return Err("could not find interrupt-out endpoint".into());
    };
    configure_endpoint(&mut device_handle, &out_endpoint)?;

    let Some(in_endpoint) = find_endpoint(
      &mut device,
      &device_desc,
      Direction::In,
      TransferType::Interrupt,
    ) else {
      return Err("could not find interrupt-in endpoint".into());
    };
    configure_endpoint(&mut device_handle, &in_endpoint)?;

    return Ok(Self {
      device_handle,
      in_endpoint,
      out_endpoint,
    });
  }

  fn get_uart_cmd(&mut self, packet: [u8; 64]) -> Result<([u8; 64], u32), Box<StdError>> {
    let mut buf = [0x00; 64];

    // Clear out anything so we can read properly.
    // Another way to solve this is to use a thread to run in a loop and
    // continually read interrupts. We then would only store data for interrupts
    // we are waiting for. Then this function can grab that data when it becomes available.
    for _ in 0..10 {
      self
        .device_handle
        .read_interrupt(self.in_endpoint.address, &mut buf, Duration::from_millis(1))
        .ok();
    }

    let timeout = Duration::from_secs(1);

    self
      .device_handle
      .write_interrupt(self.out_endpoint.address, &packet, timeout)?;

    self
      .device_handle
      .read_interrupt(self.in_endpoint.address, &mut buf, timeout)?;
    // 0x1, 0x35, 0x62, 0x30, 0x30, 0x35, 0x30, 0x30, 0x30, 0x30, 0x32, 0xd

    if buf[1] != 0x35 {
      return Err(format!("failed to read: {:x?}", buf).into());
    }

    // println!("buf: {:x?}", buf);

    // buf:
    // 0 is index
    // 1 is header
    // 2 is R/W
    // 3 ?
    // 4 ?
    // 5 ?
    // 6 ?
    // 7 ?
    // 8-10 is ascii string of the return value.
    // 11 is end of command marker (0xd)

    // const BASE_CMD_VALUE: u8 = 48;
    // let mut num = 0;
    // for i in 0..RETURN_VALUE_NUM {
    //   num +=
    //     10u32.pow(i.try_into().unwrap()) * ((buf[ON_END_INDEX - i as usize] - BASE_CMD_VALUE) as u32);
    // }

    let num =
      std::str::from_utf8(&buf[(ON_END_INDEX - (RETURN_VALUE_NUM - 1))..=ON_END_INDEX])?.parse()?;

    // println!("{:?}", buf);

    Ok((buf, num))
  }

  // pub(crate) fn get_volume(&mut self) -> Result<u32, Box<StdError>> {
  //   let packet = make_packet(&[INDEX, 0x35, 0x38, 0x30, 0x30, 0x38, 0x37, 0x30, 0xd]);
  //   let (_, value) = self.get_uart_cmd(packet)?;
  //   Ok(value)
  // }

  pub(crate) fn test(&mut self) -> Result<u32, Box<StdError>> {
    let packet = make_packet(&[INDEX, 53, 56, 48, 48, 49, 51, 48, 13]);
    let (_, value) = self.get_uart_cmd(packet)?;
    Ok(value)
  }

  pub(crate) fn get_kvm(&mut self) -> Result<u32, Box<StdError>> {
    let packet = make_packet(&[INDEX, 0x35, 0x38, 0x30, 0x30, 0x38, 0x3e, 0x30, 0xd]);
    let (_, value) = self.get_uart_cmd(packet)?;
    Ok(value)
  }

  pub(crate) fn get_input(&mut self) -> Result<u32, Box<StdError>> {
    let packet = make_packet(&[INDEX, 0x35, 0x38, 0x30, 0x30, 0x35, 0x30, 0x30, 0xd]);
    let (_, value) = self.get_uart_cmd(packet)?;
    Ok(value)
  }

  // pub(crate) fn set_volume(&mut self, level: u8) -> Result<(), Box<StdError>> {
  //   let timeout = Duration::from_secs(1);

  //   let buf = make_packet(&[
  //     INDEX,
  //     0x35,
  //     0x62,
  //     0x30,
  //     0x30,
  //     0x38,
  //     0x37,
  //     0x30,
  //     0x30,
  //     0x30,
  //     0x30 + level,
  //     0xd,
  //   ]);
  //   self
  //     .device_handle
  //     .write_interrupt(self.out_endpoint.address, &buf, timeout)?;

  //   // There is a response but we don't care about it. This is more here
  //   // for the delay so you can set input and kvm one after another. Without
  //   // this delay, setting the kvm or input right after another could fail.
  //   // Another option is to retry on failure.
  //   let mut buf = [0x00; 64];
  //   for _ in 0..5 {
  //     self
  //       .device_handle
  //       .read_interrupt(self.in_endpoint.address, &mut buf, Duration::from_millis(1))
  //       .ok();
  //   }

  //   Ok(())
  // }

  pub(crate) fn set_input(&mut self, position: u8) -> Result<(), Box<StdError>> {
    let timeout = Duration::from_secs(1);

    let buf = make_packet(&[
      INDEX,
      0x35,
      0x62,
      0x30,
      0x30,
      0x35,
      0x30,
      0x30,
      0x30,
      0x30,
      0x30 + position,
      0x0d,
    ]);
    self
      .device_handle
      .write_interrupt(self.out_endpoint.address, &buf, timeout)?;

    // There is a response but we don't care about it. This is more here
    // for the delay so you can set input and kvm one after another. Without
    // this delay, setting the kvm or input right after another could fail.
    // Another option is to retry on failure.
    let mut buf = [0x00; 64];
    for _ in 0..5 {
      self
        .device_handle
        .read_interrupt(self.in_endpoint.address, &mut buf, Duration::from_millis(1))
        .ok();
    }

    Ok(())
  }

  pub(crate) fn set_kvm(&mut self, position: u8) -> Result<(), Box<dyn Error>> {
    let timeout = Duration::from_secs(1);

    let buf = make_packet(&[
      INDEX,
      0x35,
      0x62,
      0x30,
      0x30,
      0x38,
      0x3e,
      0x30,
      0x30,
      0x30,
      0x30 + position,
      0x0d,
    ]);
    self
      .device_handle
      .write_interrupt(self.out_endpoint.address, &buf, timeout)?;

    // There is a response but we don't care about it. This is more here
    // for the delay so you can set input and kvm one after another. Without
    // this delay, setting the kvm or input right after another could fail.
    // Another option is to retry on failure.
    let mut buf = [0x00; 64];
    for _ in 0..5 {
      self
        .device_handle
        .read_interrupt(self.in_endpoint.address, &mut buf, Duration::from_millis(1))
        .ok();
    }

    Ok(())
  }
}

#[derive(Debug)]
struct Endpoint {
  config: u8,
  iface: u8,
  setting: u8,
  address: u8,
  // interval: u8,
}

pub fn make_packet(slice: &[u8]) -> [u8; 64] {
  let mut buffer = [0x00; 64];
  let n = std::cmp::min(buffer.len(), slice.len());
  buffer[0..n].copy_from_slice(&slice[0..n]);
  buffer
}

fn get_device(
  vendor_id: u16,
  product_id: u16,
) -> Result<Option<Device<GlobalContext>>, Box<StdError>> {
  for _ in 0..3 {
    for device in rusb::devices()?.iter() {
      let device_desc = device.device_descriptor()?;

      if device_desc.vendor_id() == vendor_id && device_desc.product_id() == product_id {
        return Ok(Some(device));
      }
    }
    thread::sleep(Duration::from_millis(200));
  }

  Ok(None)
}

fn configure_endpoint<T: UsbContext>(
  handle: &mut DeviceHandle<T>,
  endpoint: &Endpoint,
) -> rusb::Result<()> {
  handle.set_active_configuration(endpoint.config)?;
  #[cfg(target_os = "macos")]
  {
    handle.detach_kernel_driver(endpoint.iface)?;
  }
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
              // interval: endpoint_desc.interval(),
            });
          }
        }
      }
    }
  }

  None
}
