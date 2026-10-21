-- Run by reaper-into.sh inside REAPER (Xvfb only). Opens the plugin's floating window, runs `drop-probe --into <title>` in
-- the background (REAPER's main thread must keep running: its windows answer XDND there), waits for it, then logs the
-- project's items and tracks. Helpers come from xtask/reaper/common.lua (ENTH_LUA_DIR); the results are in ENTH_OUT.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")
local PROBE = os.getenv("DROP_PROBE")

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  show_editor(track, fx, function(drew)
    check("editor-drew", drew, "")
    data("items-before", reaper.CountMediaItems(0))
    data("tracks-before", reaper.CountTracks(0))
    local out = OUT .. "/drop-probe-into-" .. os.getenv("ENTH_TAG") .. ".txt"
    os.remove(out)
    local cmd = "( '" .. PROBE .. "' --into " .. string.format("%q", editor_title()) .. " > '" .. out
      .. ".tmp' 2>&1; echo \"exit $?\" >> '" .. out .. ".tmp'; mv '" .. out .. ".tmp' '" .. out .. "' ) &"
    os.execute(cmd)
    wait_until(function() return io.open(out, "r") ~= nil end, 30, 0.2, function(done)
      check("probe-finished", done, out)
      wait(1.0, function()
        data("items-after", reaper.CountMediaItems(0))
        data("tracks-after", reaper.CountTracks(0))
        hide_editor(track, fx)
        wait(0.5, finish)
      end)
    end)
  end)
end)()
