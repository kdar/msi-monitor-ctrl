# msi-monitor-ctrl

This program allows you to switch KVM and input for MSI monitors without Gaming Intelligence.
This is useful for switching to Linux or OSX where Gaming Intelligence is not supported.

## Why use nusb and rusb?

I attempted to use nusb but it required to install WinUSB on windows which prevents MSI's "Gaming Intelligence" app from working anymore. I only use nusb for USB hotplug and rusb for actually writing to the monitor.

## Linux dependencies

On Linux display-info requires to install `libxcb`、`libxrandr`.

## macOS setup

No kernel extension or special USB driver is required on macOS (that's specifically a
Windows problem, see above) — just a few build dependencies and two privacy permissions.

### 1. Build dependencies

```
brew install libusb pkg-config
xcode-select --install   # if not already installed
```

Then install Rust if you don't already have it:

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Apple Silicon note

If you're on an Apple Silicon Mac but your Homebrew install is the Intel (`/usr/local`)
one — e.g. because your terminal runs under Rosetta — `cargo build` will fail to link
with an error like `symbol(s) not found for architecture arm64`. This happens because
Homebrew's `libusb` is x86_64-only while `rustc` builds natively for arm64.

The fix already applied in this repo's `Cargo.toml`: `rusb` has the `vendored` feature
enabled, which builds `libusb` from source at compile time for whatever architecture
you're actually compiling for, instead of linking against a prebuilt Homebrew binary.
If you ever see that linker error again, that's the first thing to check.

### 3. Build

```
cargo build --release
```

The binary is produced at `target/release/msi-monitor-ctrl`.

### 4. macOS permissions

Two separate privacy grants are needed, both under
**System Settings → Privacy & Security**:

- **Input Monitoring** — required for raw USB/HID access to the monitor
  (`device_open`/`get_*`/`set_*` calls). Without this, you'll see
  `Error: Access denied (insufficient permissions)`.
- **Accessibility** — required for global hotkeys (`register_hotkey`) and mouse
  control (`move_mouse`, `register_screen_edge`).

Grant these to whichever terminal app you run the binary from, then fully quit and
reopen that terminal for the grant to take effect. Permissions are tied to the specific
requesting process, so running the binary from a different app/terminal may prompt again.

### 5. Find your monitor's USB vendor/product ID

The scripts in this repo are already configured with the vendor/product ID for the
monitor this was set up against, so you likely don't need to change anything. If you're
adapting this for a different monitor, find its IDs with:

```
ioreg -l -w0 | grep -B2 -A15 "idVendor"
```

or

```
system_profiler SPUSBDataType
```

Look for your monitor's entry and note its `idVendor`/`idProduct` (convert decimal to
hex if needed), then update the `device_open(vendor_id, product_id)` call in the scripts
below.

## Included scripts

- **`my_test.lua`** — read-only sanity check. Opens the device and prints the current
  KVM and input positions. Run this first on any new machine to confirm the tool can
  talk to the monitor before trying anything that changes state:
  ```
  ./target/release/msi-monitor-ctrl --cmd my_test.lua
  ```

- **`sweep_input.lua`** — discovery tool. Cycles the video input through positions 0–4
  (four seconds each), then reverts back to whatever input you started on. Use this to
  map input position numbers to physical sources on a new setup — watch the monitor
  while it runs and note what appears at each position:
  ```
  ./target/release/msi-monitor-ctrl --cmd sweep_input.lua
  ```
  Note: if a source shows nothing during the sweep, check that the machine on that
  input is actually awake — a sleeping/locked machine sends no video signal regardless
  of which input is selected.

- **`kvm_switch.lua`** — the actual daily-use script. Registers `Cmd+Shift+K` as a
  global hotkey that toggles video input between position `0` and position `1` (the
  two machines sharing this monitor). Requires root (see "macOS permissions" above —
  raw USB claim needs `sudo` on top of the Input Monitoring grant). Run it and leave it
  running in the foreground:
  ```
  sudo ./target/release/msi-monitor-ctrl --cmd kvm_switch.lua
  ```
  This only switches the video/picture — keyboard, mouse, and webcam ownership
  (KVM/USB, `Device:set_kvm`) are intentionally left alone and stay wherever they're
  currently wired/switched. We confirmed `Device:set_kvm` positions (`0`/`2` on this
  setup) work independently via manual testing (see git history), in case combined
  video+KVM switching is wanted again later — the two switches use *different*
  position numbers even though they point at the same physical machine, since the two
  machines are wired differently (one is a combined USB-C cable carrying both video
  and data, the other has video and USB on separate connectors) and the monitor
  exposes video-input and KVM/USB-host selection as fully independent commands
  regardless of physical wiring.

  If you set this up on a second machine sharing the same monitor, run the identical
  script there too — it's a toggle, not a "switch to me" command, so the same file works
  unmodified on either side.

## Running at login

Raw USB/HID access to this monitor requires root on macOS, but global hotkeys
(`register_hotkey`) require running inside your normal GUI login session — a plain
root-owned background daemon can't register hotkeys, and a plain user-level login item
can't get root. `launchd/` in this repo resolves that by prompting for your password
once per login (via the standard macOS authorization dialog, not a terminal `sudo`
prompt) and then running the tool elevated within your session:

- `launchd/login_launch.sh` — wrapper that runs `kvm_switch.lua` via
  `osascript ... with administrator privileges`.
- `launchd/com.personal.msi-monitor-ctrl.plist` — a LaunchAgent template (paths are
  placeholders — this file is intentionally not machine-specific so it's safe to
  commit/pull across machines).

To install on a given machine:

```
chmod +x launchd/login_launch.sh

# generate a machine-specific plist from the template
sed \
  -e "s|REPLACE_WITH_ABSOLUTE_PATH_TO/launchd/login_launch.sh|$(pwd)/launchd/login_launch.sh|" \
  -e "s|REPLACE_WITH_HOME|$HOME|g" \
  launchd/com.personal.msi-monitor-ctrl.plist \
  > "$HOME/Library/LaunchAgents/com.personal.msi-monitor-ctrl.plist"

launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/com.personal.msi-monitor-ctrl.plist"
```

Output/errors land in `~/Library/Logs/msi-monitor-ctrl.log`.

To remove it:

```
launchctl bootout "gui/$(id -u)/com.personal.msi-monitor-ctrl"
rm "$HOME/Library/LaunchAgents/com.personal.msi-monitor-ctrl.plist"
```

Note: a setuid-root binary was considered as an alternative to avoid the login password
prompt entirely, but was rejected — this tool's `--cmd` flag runs arbitrary,
unsandboxed Lua (full `os`/`io` access), so making the binary setuid-root would let any
local process invoke it with a different `--cmd` payload and get arbitrary code
execution as root. The password-prompt approach avoids that; it's more friction (one
prompt per login) in exchange for not permanently widening the binary's privilege.
