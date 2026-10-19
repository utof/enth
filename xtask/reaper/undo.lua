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

  local function noop() end

  -- A step is a function that takes `next` and calls it when done. `act` runs and waits `delay` seconds; `expect` polls
  -- (up to 4 s) until the plugin holds the patch (and macro 1, if given), then records the outcome: REAPER needs a
  -- moment to restore a state, and a VST3 only applies a host-set value on its next process() call.
  local function act(f, delay)
    return function(next)
      f()
      wait(delay or 0.1, next)
    end
  end
  local function expect(step, patch, macro)
    return function(next)
      local function holds() return which_patch(track, fx) == patch and (macro == nil or near(norm(track, fx, MACRO_1), macro)) end
      wait_until(holds, 4, 0.1, function(ok)
        local m = norm(track, fx, MACRO_1)
        check(step, ok, string.format("patch %s (want %s), macro 1 = %.4f%s", which_patch(track, fx), patch, m,
          macro and string.format(" (want %.2f)", macro) or ""))
        next()
      end)
    end
  end

  local steps = {
    act(function() undoable("enth: add", noop) end),
    act(function() undoable("enth: load A", function() inject_state(track, fx) end) end),
    act(function() undoable("enth: load B", function() inject_state(track, fx, "ENTH_STATE_CHUNK_FILE_B") end) end),
    act(function() reaper.TrackFX_SetParamNormalized(track, fx, MACRO_1, 0.6) end),
    expect("before-undo", "B", 0.6),
    act(function() undoable("enth: macro 1", noop) end),
    act(function() reaper.Undo_DoUndo2(0) end),
    expect("undo-macro", "B", 0.0),
    act(function() reaper.Undo_DoUndo2(0) end),
    expect("undo-load-b", "A"),
    act(function() reaper.Undo_DoUndo2(0) end),
    expect("undo-load-a", "init"),
    act(function() reaper.Undo_DoRedo2(0) end),
    expect("redo-load-a", "A"),
    act(function() reaper.Undo_DoRedo2(0) end),
    expect("redo-load-b", "B"),
    act(function() reaper.Undo_DoRedo2(0) end),
    expect("redo-macro", "B", 0.6),
    -- The plugin still takes a parameter and saves it.
    act(function() reaper.TrackFX_SetParamNormalized(track, fx, MACRO_2, 0.4) end),
    function(next)
      wait_until(function() return near(norm(track, fx, MACRO_2), 0.4) end, 4, 0.1, function(ok)
        check("works-after-undo", ok and which_patch(track, fx) == "B",
          string.format("macro 2 = %.4f, patch %s", norm(track, fx, MACRO_2), which_patch(track, fx)))
        next()
      end)
    end,
  }
  local function run(i)
    if i > #steps then return finish() end
    steps[i](function() run(i + 1) end)
  end
  run(1)
end)()
