-- undo-state-load: REAPER's undo and redo across plugin state loads. Each step is one undo point; undoing goes back to
-- the saved state of the point before. Loads of patch A (ENTH_STATE_CHUNK_FILE) and patch B (ENTH_STATE_CHUNK_FILE_B)
-- go through the same `clap_chunk` / `vst_chunk` setting as `inject_state`, which is how REAPER hands a plugin a
-- state when it loads a project or undoes. Checks the patch the plugin holds (by its name in the saved state) and
-- macro 1 after every step, and that the plugin still works at the end.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local MACRO_1, MACRO_2 = 0, 1
local MARK_A, MARK_B = "reaper-test state", "reaper-test roundtrip"

local function near(a, b) return math.abs(a - b) < 0.002 end

local function which_patch(track, fx)
  local bytes = state_bytes(track, fx)
  if contains(bytes, MARK_B) then return "B" end
  if contains(bytes, MARK_A) then return "A" end
  return "init"
end

-- One undo point: the change runs between Begin and End, as scripts do it.
local function undoable(name, change)
  reaper.Undo_BeginBlock2(0)
  change()
  reaper.Undo_EndBlock2(0, name, -1)
end

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end

  local function state_is(step, patch, macro)
    local got, m = which_patch(track, fx), norm(track, fx, MACRO_1)
    local ok = got == patch and (macro == nil or near(m, macro))
    check(step, ok, string.format("patch %s (want %s), macro 1 = %.4f%s", got, patch, m,
      macro and string.format(" (want %.2f)", macro) or ""))
  end
  local function noop() end

  -- Each step runs, then the script waits `delay` seconds before the next: REAPER needs a moment to restore a state, and
  -- a VST3 only applies a host-set value on its next process() call.
  local steps = {
    { function() undoable("enth: add", noop) end, 0.1 },
    { function() undoable("enth: load A", function() inject_state(track, fx) end) end, 0.3 },
    { function() undoable("enth: load B", function() inject_state(track, fx, "ENTH_STATE_CHUNK_FILE_B") end) end, 0.3 },
    { function() reaper.TrackFX_SetParamNormalized(track, fx, MACRO_1, 0.6) end, 1.0 },
    { function() undoable("enth: macro 1", noop) end, 0.1 },
    { function() state_is("before-undo", "B", 0.6) end, 0 },
    { function() reaper.Undo_DoUndo2(0) end, 1.0 },
    { function() state_is("undo-macro", "B", 0.0) end, 0 },
    { function() reaper.Undo_DoUndo2(0) end, 1.0 },
    { function() state_is("undo-load-b", "A") end, 0 },
    { function() reaper.Undo_DoUndo2(0) end, 1.0 },
    { function() state_is("undo-load-a", "init") end, 0 },
    { function() reaper.Undo_DoRedo2(0) end, 1.0 },
    { function() state_is("redo-load-a", "A") end, 0 },
    { function() reaper.Undo_DoRedo2(0) end, 1.0 },
    { function() state_is("redo-load-b", "B") end, 0 },
    { function() reaper.Undo_DoRedo2(0) end, 1.0 },
    { function() state_is("redo-macro", "B", 0.6) end, 0 },
    -- The plugin still takes a parameter and saves it.
    { function() reaper.TrackFX_SetParamNormalized(track, fx, MACRO_2, 0.4) end, 1.0 },
    { function()
      check("works-after-undo", near(norm(track, fx, MACRO_2), 0.4) and which_patch(track, fx) == "B",
        string.format("macro 2 = %.4f, patch %s", norm(track, fx, MACRO_2), which_patch(track, fx)))
    end, 0 },
  }
  local function run(i)
    if i > #steps then return finish() end
    steps[i][1]()
    wait(steps[i][2], function() run(i + 1) end)
  end
  run(1)
end)()
