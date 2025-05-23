local callback = function()
  local dev = device_open(0x1462, 0x3fa4)
  dev:set_input(3)
  dev:set_kvm(2)
end

local error_handler = function(err)
  print("ERROR:", err)
  msgbox("Error", tostring(err), "error", {Ok={}})
  os.exit(1)
end

xpcall(register_hotkey, error_handler, "shift+control+alt+ArrowRight", callback)

main_loop()
