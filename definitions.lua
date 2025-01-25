---@meta

---@param duration integer
---@return nil
function sleep(duration) end

---@class Device
local Device = {}

---@return Device
function open() end

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
