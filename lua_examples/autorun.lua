local error_handler = function(err)
  print("ERROR:", err)
  msgbox("title", "message", "error", {Ok={}})
  os.exit(1)
end

xpcall(autorun, error_handler)

-- Or
-- xpcall(autorun, error_handler, "command.exe --with --args")