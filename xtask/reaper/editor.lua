-- Smoke test for the editor: open and close it five times, then leave it open for a moment, all under Xvfb. It proves
-- that a window appears and draws something (a blank window counts as a failure), that closing it removes the window,
-- and that REAPER survives. A bigger or slower editor costs time, not a false failure: each open waits (up to 15 s)
-- for the first drawn frame instead of sleeping a fixed time. How it looks and feels on the real display is the owner
-- checklist.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local CYCLES = 5

-- Whether a viewable window with the plugin's title exists (xinput `find` fails with status 2 when none does).
local function window_viewable()
  local ok, text = xinput("client " .. string.format("%q", editor_title()))
  return ok and text ~= ""
end

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  inject_state(track, fx)
  local opened, closed, slowest = 0, 0, 0
  local function cycle(n)
    if n > CYCLES then
      check("editor-opens", opened == CYCLES, opened .. "/" .. CYCLES .. " opens drew an editor")
      check("editor-closes", closed == CYCLES, closed .. "/" .. CYCLES .. " closes removed the window")
      data("editor-slowest-first-frame-s", string.format("%.2f", slowest))
      return show_editor(track, fx, function(rendered)
        wait(2.0, function()
          local ok, text = xinput("stats " .. string.format("%q", editor_title()))
          local still = reaper.TrackFX_GetFloatingWindow(track, fx) ~= nil and ok
          check("editor-stays-open", rendered and still, "window still drawing after 2 s: " .. text)
          hide_editor(track, fx)
          wait(0.5, finish)
        end)
      end)
    end
    show_editor(track, fx, function(rendered, seconds)
      if rendered then opened = opened + 1 end
      if seconds > slowest then slowest = seconds end
      hide_editor(track, fx)
      wait_until(function() return not window_viewable() end, 5, 0.2, function(gone)
        if gone then closed = closed + 1 end
        cycle(n + 1)
      end)
    end)
  end
  cycle(1)
end)()
