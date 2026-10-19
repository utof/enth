-- Smoke test for the placeholder editor: open and close it five times, then leave it open for a
-- moment, all under Xvfb. It only proves the window appears and REAPER survives; how it looks and
-- feels on the real display is the owner checklist.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local CYCLES = 5

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  local opened = 0
  local function cycle(n)
    if n > CYCLES then
      check("editor-opens", opened == CYCLES, opened .. "/" .. CYCLES .. " opens showed a window")
      reaper.TrackFX_Show(track, fx, 3)
      return wait(2.0, function()
        check("editor-stays-open", reaper.TrackFX_GetFloatingWindow(track, fx) ~= nil, "window still there after 2 s")
        reaper.TrackFX_Show(track, fx, 2)
        wait(0.5, finish)
      end)
    end
    reaper.TrackFX_Show(track, fx, 3) -- show floating window
    wait(0.8, function()
      if reaper.TrackFX_GetFloatingWindow(track, fx) ~= nil then opened = opened + 1 end
      reaper.TrackFX_Show(track, fx, 2) -- hide floating window
      wait(0.3, function() cycle(n + 1) end)
    end)
  end
  cycle(1)
end)()
