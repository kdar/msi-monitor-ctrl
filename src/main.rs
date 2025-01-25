use std::{thread, time::Duration};

use clap::Parser;
use errors::StdError;
use mlua::Lua;

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
  // }

  fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
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
  let args = Args::parse();

  let lua = Lua::new();

  let open = lua.create_function(|_, ()| -> Result<device::MSIDevice, mlua::Error> {
    let dev = device::MSIDevice::open().map_err(mlua::ExternalError::into_lua_err)?;
    Ok(dev)
  })?;

  let sleep = lua.create_function(|_, duration: u64| -> Result<(), mlua::Error> {
    thread::sleep(Duration::from_millis(duration));
    Ok(())
  })?;

  lua.globals().set("open", &open)?;
  lua.globals().set("sleep", &sleep)?;

  lua.load(args.cmd).exec()?;

  Ok(())
}
