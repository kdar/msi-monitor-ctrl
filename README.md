# msi-monitor-ctrl

This program allows you to switch KVM and input for MSI monitors without Gaming Intelligence.
This is useful for switching to Linux or OSX where Gaming Intelligence is not supported.

This is more of a proof of concept for my uses only. It hardcodes my setup and assumptions. If other people find this useful, I may make it more generic and configurable.

## Why use nusb and rusb?

I attempted to use nusb but it required to install WinUSB on windows which prevents MSI's "Gaming Intelligence" app from working anymore. I only use nusb for USB hotplug and rusb for actually writing to the monitor.
