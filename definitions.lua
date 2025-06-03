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

---@param x number Absolute or relative x position.
---@param y number Absolute or relative y position.
---@param moving_time number Floating number for how long to take moving the mouse.
---@param mode string "rel" or "abs" to move relatively or absolutely.
---@return nil
function move_mouse(x, y, moving_time, mode) end

---@return number ... Width and height
function screen_size() end

---@param lo_interval number Low interval time in milliseconds.
---@param hi_interval number High interval time in milliseconds.
---@param callback function   
---@return number id An ID you can use to call unregister_interval.
---Register an interval on which to execute a function. A random
---number will be chosen between lo_interval and hi_interval. If
---they are equal values, it will always use that value to execute
---the function.
function register_interval(lo_interval, hi_interval, callback) end

---@param id number The ID returned from register_interval. 
---@return nil
function unregister_interval(id) end

---@class Device
local Device = {}

---@param vendor_id integer
---@param product_id integer
---@return Device
function device_open(vendor_id, product_id) end

---@param vendor_id integer
---@param product_id integer
---@return boolean
function device_is_connected(vendor_id, product_id) end

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
