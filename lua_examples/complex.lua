local VENDOR_ID = 0x1462
local PRODUCT_ID = 0x3fa4

local hotkey_callback = function(hk)
  local dev = open(VENDOR_ID, PRODUCT_ID)
  if host_os == "windows" then
    dev:set_input(3)
    sleep_ms(100)
    dev:set_kvm(2)
  else
    dev:set_input(2)
    sleep_ms(100)
    dev:set_kvm(1)
  end
end

local hotplug_callback = function(status, vendor_id, product_id)
  if vendor_id ~= VENDOR_ID or product_id ~= PRODUCT_ID then
    return
  end

  if status == "connected" then
    os.execute('displayplacer "id:3145D954-9166-4630-537F-6A7A36E2B478 res:1792x1120 hz:59 color_depth:4 enabled:true scaling:on origin:(0,0) degree:0" "id:3FDCE03B-E3E4-24D1-82FE-C00ABF19A2B0 res:2560x1440 hz:59 color_depth:8 enabled:true scaling:off origin:(-2560,0) degree:0"')
  else
    os.execute('displayplacer "id:3FDCE03B-E3E4-24D1-82FE-C00ABF19A2B0 enabled:false"')
  end
end

local error_handler = function(err)
  print("ERROR:", err)
  os.exit(1)
end

if host_os == "macos" then
  xpcall(register_hotkey, error_handler, "shift+super+alt+ArrowRight", hotkey_callback)
  xpcall(register_hotplug, error_handler, hotplug_callback)
else
  xpcall(register_hotkey, error_handler, "shift+control+alt+ArrowRight", hotkey_callback)
end

main_loop()
