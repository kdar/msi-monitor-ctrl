local dev = device_open(0x1462, 0x3fa4)

local THIS_INPUT = 0
local OTHER_INPUT = 1
local THIS_KVM = 0
local OTHER_KVM = 2

register_hotkey("CMD+SHIFT+K", function()
  local current_input = dev:get_input()
  local target_input = THIS_INPUT
  local target_kvm = THIS_KVM
  if current_input == THIS_INPUT then
    target_input = OTHER_INPUT
    target_kvm = OTHER_KVM
  end

  print("switching input from " .. current_input .. " to " .. target_input)
  dev:set_input(target_input) -- must happen before kvm, or we may lose USB access

  print("switching kvm to " .. target_kvm)
  dev:set_kvm(target_kvm)
end)

print("ready - press Cmd+Shift+K to switch input+kvm between the two laptops")
main_loop()
