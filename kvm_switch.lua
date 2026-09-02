local dev = device_open(0x1462, 0x3fa4)

local THIS_INPUT = 0
local OTHER_INPUT = 1

register_hotkey("CMD+SHIFT+K", function()
  local current_input = dev:get_input()
  local target_input = THIS_INPUT
  if current_input == THIS_INPUT then
    target_input = OTHER_INPUT
  end

  print("switching input from " .. current_input .. " to " .. target_input)
  dev:set_input(target_input)
end)

print("ready - press Cmd+Shift+K to switch the display between the two laptops")
main_loop()
