local dev = device_open(0x1462, 0x3fa4)

register_hotkey("CMD+SHIFT+K", function()
  local current = dev:get_input()
  local target = 0
  if current == 0 then
    target = 1
  end
  print("switching input from " .. current .. " to " .. target)
  dev:set_input(target)
end)

print("ready - press Cmd+Shift+K to switch input between the two laptops")
main_loop()
