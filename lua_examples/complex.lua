local VENDOR_ID = 0x1462
local PRODUCT_ID = 0x3fa4

local error_handler = function(err)
  print("ERROR:", err)
  msgbox("title", "message", "error", {Ok={}})
  os.exit(1)
end

local hotkey_callback = function(hk)
  local ok, dev = xpcall(open, error_handler, VENDOR_ID, PRODUCT_ID)
  if not ok then
    return
  end

  -- If we are on windows, then set the input to 3 which is "Type C" and
  -- KVM to 2 which is "Type C". If not windows, set input to 2 which is
  -- "DP" and KVM to 1 which is "Upstream".
  if host_os == "windows" then
    dev:set_input(3)
    dev:set_kvm(2)
  else
    dev:set_input(2)
    dev:set_kvm(1)
  end
end

local hotplug_callback = function(status, vendor_id, product_id)
  -- We only care about our specific monitor.
  if vendor_id ~= VENDOR_ID or product_id ~= PRODUCT_ID then
    return
  end

  -- If we are connected on macos, then set it up such that the external monitor is
  -- an extension of the macbook monitor. If we disconnect, then remove the
  -- external monitor. This allows the windows in macos to be on both screens when
  -- the external monitor is connected, and all the windows to be on only the macbook
  -- screen when it is disconnected.
  if status == "connected" then
    os.execute('displayplacer "id:3145D954-9166-4630-537F-6A7A36E2B478 res:1792x1120 hz:59 color_depth:4 enabled:true scaling:on origin:(0,0) degree:0" "id:3FDCE03B-E3E4-24D1-82FE-C00ABF19A2B0 res:2560x1440 hz:59 color_depth:8 enabled:true scaling:off origin:(-2560,0) degree:0"')
  else
    os.execute('displayplacer "id:3FDCE03B-E3E4-24D1-82FE-C00ABF19A2B0 enabled:false"')
  end
end

if host_os == "macos" then
  -- On macos I have the cmd and ctrl swapped to make it more natural to use the same
  -- windows-based keyboard on both systems. So I set a specific macos hotkey here.
  xpcall(register_hotkey, error_handler, "shift+super+alt+ArrowRight", hotkey_callback)
  -- On macos we detect when the monitor is plugged in so we can use `displayplacer` to
  -- set up our screens how we like.
  xpcall(register_hotplug, error_handler, hotplug_callback)
else
  xpcall(register_hotkey, error_handler, "shift+control+alt+ArrowRight", hotkey_callback)
end

-- Run the main loop to listen for hotkeys and hotplug events.
main_loop()
