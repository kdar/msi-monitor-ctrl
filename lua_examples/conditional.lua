local dev = device_open(0x1462, 0x3fa4)
if dev:get_input() == 2 then
  dev:set_input(3)
else
  dev:set_input(2)
end
-- sleep_ms(500)
if dev:get_kvm() == 1 then
  dev:set_kvm(2)
else
  dev:set_kvm(1)
end