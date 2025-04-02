---@meta

---@type string
_G.host_os = ""

---@type string
_G.host_arch = ""

---@type string
_G.host_family = ""

---@param duration integer
---@return nil
function sleep_ms(duration) end

---@param hotkey string
---@param callback function   
---@return nil
function register_hotkey(hotkey, callback) end

---@param callback function
---@return nil
function register_hotplug(hotkey, callback) end

---@return nil
function main_loop() end

---@param title string
---@param message string
---@param level string
---@param buttoncfg table
---@return nil
function msgbox(title, message, level, buttoncfg) end

---@param app_path string? Optional app path to run. If not provided, will use what the command was run with.
---@param args string[]? Optional args to use. If not provided, will use what the command was run with.
---@return nil
function autorun(cmd, app_path, args) end

---@class Device
local Device = {}

---@param vendor_id integer
---@param product_id integer
---@return Device
function open(vendor_id, product_id) end

---@param self self
---@return integer
function Device:get_kvm() end

---@param self self
---@param position integer
---@return nil
function Device:set_kvm(position) end

---@param self self
---@return integer
function Device:get_input() end

---@param self self
---@param position integer
---@return nil
function Device:set_input(position) end
