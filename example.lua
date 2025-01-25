local dev = open()
if dev:get_input() == 2 then
  dev:set_input(3)
else
  dev:set_input(2)
end
-- sleep(500)
if dev:get_kvm() == 1 then
  dev:set_kvm(2)
else
  dev:set_kvm(1)
end