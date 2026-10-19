-- dirty-flag: does REAPER mark the project dirty after an edit made in the plugin's editor? (Open check, phase-1b.md:
-- nice-plug never calls CLAP `mark_dirty` or VST3 `setDirty`.)
--   edit:    a patch edit nothing else sees: no host parameter is bound to the dragged knob.
--   control: the same drag on a knob that is bound to a host slot; the editor then writes the host parameter
--            (begin, set, end gesture), which hosts treat as a change.
--   idle:    no edit at all, only the editor opened and closed: what the project reads when nothing was edited.
-- After the drag the flag is read at once, 3 s later, and after the editor window was closed.
-- Each phase loads its patch, opens the editor, saves the project (so it is clean), drags a knob with real X events
-- (xinput.py, under Xvfb only) and reads `IsProjectDirty`. The drag looks for a knob: the first point is where the F2b
-- editor's "Level" knob sits (an offset into the plugin's window); if the saved state does not change there, a coarse
-- grid over the window is tried until a drag changes it, so a new layout is found, not failed.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local phase = os.getenv("ENTH_PHASE")
local tag = os.getenv("ENTH_TAG")
local project = OUT .. "/dirty-" .. phase .. "-" .. tag .. ".rpp"
local FIRST_POINT = { 366, 64 } -- offset of F2b's "Level" knob inside the plugin window
do -- ENTH_EDIT_POINT=x,y overrides it (to try the search with a point that is not on a knob)
  local x, y = (os.getenv("ENTH_EDIT_POINT") or ""):match("^(%d+),(%d+)$")
  if x then FIRST_POINT = { tonumber(x), tonumber(y) } end
end
local BUDGET = 40 -- seconds for the grid search

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

-- Drags down 30 px at (px, py) in the window, then up 30 px if nothing moved (a knob at its minimum).
local function try_drag(track, fx, before, x, y, px, py)
  for _, dy in ipairs({ 30, -30 }) do
    xinput(string.format("drag %d %d %d %d", x + px, y + py, x + px, y + py + dy))
    sleep(0.25) -- the GUI thread needs a frame to apply the edit
    if state_chunk(track, fx) ~= before then return true end
  end
  return false
end

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  inject_state(track, fx)
  show_editor(track, fx, function(drew)
    check(phase .. "-editor-drew", drew, "the editor drew before the drag")
    if not drew then return finish() end
    reaper.Main_SaveProjectEx(0, project, 0)
    local before = state_chunk(track, fx)
    data("dirty-after-save", reaper.IsProjectDirty(0))
    check(phase .. "-clean-after-save", reaper.IsProjectDirty(0) == 0, "IsProjectDirty after saving the project")
    local changes_before = reaper.GetProjectStateChangeCount(0)
    local params_before = snapshot(track, fx)
    local x, y, w, h = window()
    if not x then check(phase .. "-editor-window", false, "no plugin window found") return finish() end
    data("editor-window", string.format("%d,%d %dx%d", x, y, w, h))

    -- The knob search is a plain loop: each drag is a blocking call anyway. A short wait after each lets the GUI thread
    -- draw a frame (and apply the edit) before the state is read. The idle phase makes no edit at all.
    local found
    if phase == "idle" then
      sleep(0.5)
      found = false
    elseif try_drag(track, fx, before, x, y, FIRST_POINT[1], FIRST_POINT[2]) then
      found = FIRST_POINT
    else
      local t0 = reaper.time_precise()
      for py = 40, h - 40, 36 do
        for px = 24, w - 24, 36 do
          if reaper.time_precise() - t0 > BUDGET then break end
          if try_drag(track, fx, before, x, y, px, py) then found = { px, py } break end
        end
        if found or reaper.time_precise() - t0 > BUDGET then break end
      end
    end
    if phase == "idle" then
      check("idle-state-unchanged", state_chunk(track, fx) == before, "no edit, so the saved state is the same")
    else
      check(phase .. "-edit-applied", found ~= nil, found and ("a drag at " .. found[1] .. "," .. found[2] .. " changed the saved state")
        or "no drag changed the saved state: the editor's layout moved? update FIRST_POINT in dirty.lua")
    end
    wait(0.5, function()
      data("host-params-moved", moved(params_before, snapshot(track, fx)))
      data("dirty-after-edit", reaper.IsProjectDirty(0))
      data("state-changes-during-edit", reaper.GetProjectStateChangeCount(0) - changes_before)
      data("edit-point", type(found) == "table" and (found[1] .. "," .. found[2]) or "none")
      -- REAPER might notice a plugin's change later (a poll, or when the window closes): look again, 2.5 s later (the
      -- phases that only compare the flag after closing the window skip that wait).
      wait(phase == "edit" and 2.5 or 0, function()
        data("dirty-late", reaper.IsProjectDirty(0))
        hide_editor(track, fx)
        wait(0.7, function()
          data("dirty-after-close", reaper.IsProjectDirty(0))
          finish()
        end)
      end)
    end)
  end)
end)()
