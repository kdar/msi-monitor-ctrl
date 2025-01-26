local dev = open(0x1462, 0x3fa4)
-- Must set the input before we switch KVM. If we switch the KVM first, we
-- will lose USB access to the monitor.
dev:set_input(3)
dev:set_kvm(2)