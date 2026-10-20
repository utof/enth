-- preset-load (lane PBU): a preset loaded through the plugin's own browser, with real X events (xinput.py, under Xvfb
-- only). The project starts from the `state` patch ("reaper-test state"), saved so it is clean; then the editor's
-- Browse, its User tab and the first row (the one user preset in ENTH_PRESETS_DIR, "reaper-test roundtrip") are clicked.
-- Checks:
--   loaded:        the plugin's saved state holds the preset's patch and the browser's bookmark (user:roundtrip);
--   no host write: all 72 host parameters read the same before and after (the preset's macros and slot targets differ
--                  from the state patch's, so a per-parameter write storm would move them: nih-plug #92);
--   dirty:         REAPER marks the project dirty (fork patch 6, the session's PatchEdited);
--   one undo step: one click on the editor's Undo brings the whole state patch back, one Redo the preset, still with
--                  no host write.
-- Then the open check of phase-2.md: ▶ held on the loaded row with the transport stopped, and the track's peak meter
-- read while it is held (data, not a check: whether REAPER keeps calling process() while stopped).
--
-- The click points are offsets into the plugin's window (layout "lanes" at 1200 x 800). `reaper_points_hit_their_controls`
-- in crates/enth-ui/tests/browser.rs asserts the same numbers against the editor, so a layout change fails there first.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local POINTS = {
  browse = { 347, 28 },
  user_tab = { 138, 80 },
  first_row = { 200, 140 },
  first_play = { 60, 140 },
  undo = { 759, 28 },
  redo = { 809, 28 },
}
local MARK_STATE, MARK_PRESET = "reaper-test state", "reaper-test roundtrip"
-- The browser's bookmark in the `enth` field (`"preset":{"last":"user:roundtrip"}`; quotes may be escaped in the chunk).
local BOOKMARK = "user:roundtrip"

local function snapshot(track, fx)
  local t = {}
  for i = 0, 71 do t[i] = norm(track, fx, i) end
  return t
end

local function moved(a, b)
  local n = 0
  for i = 0, 71 do
    if math.abs(a[i] - b[i]) > 1e-6 then n = n + 1 end
  end
  return n
end

local function window()
  local ok, text = xinput("client " .. string.format("%q", editor_title()))
  if not ok then return nil end
  local x, y, w, h = text:match("^%d+ (%-?%d+) (%-?%d+) (%d+) (%d+)$")
  if not x then return nil end
  return tonumber(x), tonumber(y), tonumber(w), tonumber(h)
end

local function holds(track, fx, mark)
  return contains(state_bytes(track, fx), mark)
end

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  inject_state(track, fx)
  show_editor(track, fx, function(drew)
    check("editor-drew", drew, "the editor drew before the clicks")
    if not drew then return finish() end
    local project = OUT .. "/preset-load-" .. os.getenv("ENTH_TAG") .. ".rpp"
    reaper.Main_SaveProjectEx(0, project, 0)
    check("clean-after-save", reaper.IsProjectDirty(0) == 0, "IsProjectDirty after saving the project")
    check("starts-from-state", holds(track, fx, MARK_STATE), "the state patch is loaded")
    local params = snapshot(track, fx)
    local x, y, w, h = window()
    if not x then
      check("editor-window", false, "no plugin window found")
      return finish()
    end
    data("editor-window", string.format("%d,%d %dx%d", x, y, w, h))
    local function click(name)
      local p = POINTS[name]
      xinput(string.format("click %d %d", x + p[1], y + p[2]))
    end

    click("browse")
    sleep(0.4)
    click("user_tab")
    sleep(0.4)
    local changes = reaper.GetProjectStateChangeCount(0)
    click("first_row")
    wait_until(function() return holds(track, fx, MARK_PRESET) end, 4, 0.1, function(loaded)
      check("preset-loaded", loaded, loaded and "the saved state holds the preset's patch"
        or "no load: the click points moved? see reaper_points_hit_their_controls in crates/enth-ui/tests/browser.rs")
      if not loaded then return finish() end
      check("preset-bookmark", holds(track, fx, BOOKMARK), "the instance state names the preset (" .. BOOKMARK .. ")")
      check("preset-no-host-writes", moved(params, snapshot(track, fx)) == 0,
        moved(params, snapshot(track, fx)) .. " of 72 host parameters moved")
      -- The fork posts the VST3 setDirty to the host's run loop, which REAPER services when its main loop is free (a
      -- fixed 0.5 s wait read 0 in two runs of four), so wait for the flag, without blocking REAPER's main thread.
      local t_load = reaper.time_precise()
      wait_until(function() return reaper.IsProjectDirty(0) == 1 end, 5, 0.05, function(dirty)
        check("preset-dirty", dirty, string.format("IsProjectDirty = %d, %.2f s after the load",
          reaper.IsProjectDirty(0), reaper.time_precise() - t_load))
        data("dirty-delay-s", string.format("%.2f", reaper.time_precise() - t_load))
        data("state-changes-during-load", reaper.GetProjectStateChangeCount(0) - changes)

        click("undo")
        wait_until(function() return holds(track, fx, MARK_STATE) end, 4, 0.1, function(undone)
          check("undo-one-step", undone and not holds(track, fx, MARK_PRESET),
            "one Undo brings the state patch back")
          check("undo-no-host-writes", moved(params, snapshot(track, fx)) == 0,
            moved(params, snapshot(track, fx)) .. " of 72 host parameters moved")
          click("redo")
          wait_until(function() return holds(track, fx, MARK_PRESET) end, 4, 0.1, function(redone)
            check("redo-one-step", redone, "one Redo brings the preset back")
            check("redo-no-host-writes", moved(params, snapshot(track, fx)) == 0,
              moved(params, snapshot(track, fx)) .. " of 72 host parameters moved")

            -- The open check: hold ▶ on the loaded row (no second load) with the transport stopped.
            data("transport-playing", reaper.GetPlayState() & 1)
            local p = POINTS.first_play
            xinput(string.format("down %d %d", x + p[1], y + p[2]))
            local peak, t0 = 0, reaper.time_precise()
            local poll
            poll = safe(function()
              for ch = 0, 1 do peak = math.max(peak, reaper.Track_GetPeakInfo(track, ch)) end
              if reaper.time_precise() - t0 < 1.0 then return reaper.defer(poll) end
              xinput(string.format("up %d %d", x + p[1], y + p[2]))
              data("audition-peak", string.format("%.6f", peak))
              data("audition-sounds-while-stopped", peak > 0 and 1 or 0)
              check("audition-no-load", holds(track, fx, MARK_PRESET), "▶ on the loaded row loads nothing else")
              hide_editor(track, fx)
              wait(0.5, finish)
            end)
            reaper.defer(poll)
          end)
        end)
      end)
    end)
  end)
end)()
