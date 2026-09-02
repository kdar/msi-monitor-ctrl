local dev = device_open(0x1462, 0x3fa4)
print("current kvm position:", dev:get_kvm())
print("current input position:", dev:get_input())
