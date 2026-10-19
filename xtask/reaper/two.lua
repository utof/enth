-- two-instances: two instances of the plugin hold independent state.
--   create: track 1 gets patch A (ENTH_STATE_CHUNK_FILE), track 2 patch B (ENTH_STATE_CHUNK_FILE_B); macro 1 and slot 1
--           get different values on each; moving one instance must not move the other; save the project.
--   reload: new process: each instance comes back with its own patch and values; both editors open at the same time and
--           draw; deleting one instance leaves the other's state byte-identical.
-- The xtask decodes the dumped states (patch names, folded host values).
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local phase = os.getenv("ENTH_PHASE")
local tag = os.getenv("ENTH_TAG")
local project = OUT .. "/two-" .. tag .. ".rpp"
local MACRO_1, SLOT_1 = 0, 8
local WANT = { a = { macro = 0.2, slot = 0.1 }, b = { macro = 0.9, slot = 0.8 } }

local function near(a, b) return math.abs(a - b) < 0.002 end

local function report(name, track, fx, want)
  local m, s = norm(track, fx, MACRO_1), norm(track, fx, SLOT_1)
  check(name, near(m, want.macro) and near(s, want.slot),
    string.format("macro 1 = %.4f (want %.2f), slot 1 = %.4f (want %.2f)", m, want.macro, s, want.slot))
end

if phase == "create" then
  safe(function()
    local ta, fa = new_project_with_fx()
    if fa < 0 then return finish() end
    reaper.InsertTrackAtIndex(1, true)
    local tb = reaper.GetTrack(0, 1)
    local fb = reaper.TrackFX_AddByName(tb, os.getenv("ENTH_FX"), false, -1)
    check("load-second", fb >= 0, "second instance on track 2")
    if fb < 0 then return finish() end
    inject_state(ta, fa)
    inject_state(tb, fb, "ENTH_STATE_CHUNK_FILE_B")
    reaper.TrackFX_SetParamNormalized(ta, fa, MACRO_1, WANT.a.macro)
    reaper.TrackFX_SetParamNormalized(ta, fa, SLOT_1, WANT.a.slot)
    reaper.TrackFX_SetParamNormalized(tb, fb, MACRO_1, WANT.b.macro)
    reaper.TrackFX_SetParamNormalized(tb, fb, SLOT_1, WANT.b.slot)
    -- VST3 only applies a host-set value on the next process() call, so wait before reading it back.
    wait(1.5, function()
      report("values-a", ta, fa, WANT.a)
      report("values-b", tb, fb, WANT.b)
      dump_chunk("two-a-create", ta, fa)
      dump_chunk("two-b-create", tb, fb)
      reaper.Main_SaveProjectEx(0, project, 0)
      check("project-saved", reaper.file_exists(project), project)
      -- Moving B must not move A: its parameters and its saved state stay as they were.
      local before = state_chunk(ta, fa)
      reaper.TrackFX_SetParamNormalized(tb, fb, MACRO_1, 0.5)
      wait(1.0, function()
        check("b-moved", near(norm(tb, fb, MACRO_1), 0.5), string.format("macro 1 of B = %.4f", norm(tb, fb, MACRO_1)))
        check("a-unmoved", near(norm(ta, fa, MACRO_1), WANT.a.macro) and state_chunk(ta, fa) == before,
          "A's macro 1 and its saved state are unchanged after B moved")
        finish()
      end)
    end)
  end)()
else
  safe(function()
    reaper.Main_openProject("noprompt:" .. project)
    local ta, tb = reaper.GetTrack(0, 0), reaper.GetTrack(0, 1)
    wait(1.5, function()
      check("reload-fx", ta ~= nil and tb ~= nil and reaper.TrackFX_GetCount(ta) == 1 and reaper.TrackFX_GetCount(tb) == 1,
        "one plugin on each of two tracks")
      if not (ta and tb) then return finish() end
      report("reload-values-a", ta, 0, WANT.a)
      report("reload-values-b", tb, 0, WANT.b)
      dump_chunk("two-a-reload", ta, 0)
      dump_chunk("two-b-reload", tb, 0)
      -- Both editors open at once.
      show_editor(ta, 0, function(drew_a)
        show_editor(tb, 0, function(drew_b)
          check("both-editors-draw", drew_a and drew_b, "A: " .. tostring(drew_a) .. ", B: " .. tostring(drew_b))
          hide_editor(ta, 0)
          hide_editor(tb, 0)
          wait(0.5, function()
            -- Deleting A leaves B alone.
            local before = state_chunk(tb, 0)
            reaper.DeleteTrack(ta)
            wait(1.0, function()
              local survivor = reaper.GetTrack(0, 0)
              check("b-survives-delete",
                survivor ~= nil and reaper.TrackFX_GetCount(survivor) == 1 and state_chunk(survivor, 0) == before
                  and near(norm(survivor, 0, MACRO_1), WANT.b.macro),
                "B's saved state is byte-identical after A's track was deleted")
              finish()
            end)
          end)
        end, editor_title() .. " - Track 2")
      end, editor_title() .. " - Track 1")
    end)
  end)()
end
