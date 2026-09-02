local dev = device_open(0x1462, 0x3fa4)

local original = dev:get_input()
print("starting input position: " .. original)
print("")

for i = 0, 4 do
  print("setting input to position " .. i .. " -- look at the monitor now")
  dev:set_input(i)
  sleep_ms(4000)
end

print("")
print("reverting to original input position: " .. original)
dev:set_input(original)
