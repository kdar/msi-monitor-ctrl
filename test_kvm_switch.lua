local dev = device_open(0x1462, 0x3fa4)

local input = dev:get_input()
local kvm = dev:get_kvm()
print("current input position: " .. input)
print("current kvm position: " .. kvm)

local target = 0
if input == 0 then
  target = 1
end

print("switching input to " .. target .. " (must happen before kvm)")
dev:set_input(target)

sleep_ms(500)

print("switching kvm to " .. target .. " -- this may hand off control to the other machine now")
dev:set_kvm(target)

print("done. if control just moved away from this machine, further device_open calls here will fail until it's switched back.")
