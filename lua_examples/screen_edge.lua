local error_handler = function(err)
  print("ERROR:", err)
  msgbox("Error", tostring(err), "error", {Ok={}})
  os.exit(1)
end

-- Argument is one of: "n", "s", "w", "e", "ne", "nw", "se", "sw".
-- The callback fires once when the mouse enters an edge/corner; it does
-- not repeat while the mouse stays in the same zone.
local edge_callback = function(edge)
  print("Mouse touched screen edge:", edge)
end

xpcall(register_screen_edge, error_handler, edge_callback)

main_loop()
