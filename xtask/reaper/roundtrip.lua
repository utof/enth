-- patch-roundtrip: a non-default patch survives a project save and a reload in a new REAPER process, byte for byte.
--   create:  load the patch as state, dump the saved state, save the project (a).
--   reload1: new process: open (a), dump the saved state, save the project again as (b).
--   reload2: new process: open (b), dump the saved state.
-- The xtask decodes the three dumps and compares the patch text, the instance state and the parameters.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local phase = os.getenv("ENTH_PHASE")
local tag = os.getenv("ENTH_TAG")
local project_a = OUT .. "/roundtrip-" .. tag .. "-a.rpp"
local project_b = OUT .. "/roundtrip-" .. tag .. "-b.rpp"

if phase == "create" then
  safe(function()
    local track, fx = new_project_with_fx()
    if fx < 0 then return finish() end
    inject_state(track, fx)
    wait(1.0, function()
      dump_chunk("roundtrip-create", track, fx)
      reaper.Main_SaveProjectEx(0, project_a, 0)
      check("project-saved-a", reaper.file_exists(project_a), project_a)
      finish()
    end)
  end)()
else
  safe(function()
    reaper.Main_openProject("noprompt:" .. (phase == "reload1" and project_a or project_b))
    local track = reaper.GetTrack(0, 0)
    wait(1.0, function()
      check(phase .. "-fx", track ~= nil and reaper.TrackFX_GetCount(track) == 1, "one plugin on track 1")
      if not track then return finish() end
      dump_chunk("roundtrip-" .. phase, track, 0)
      if phase == "reload1" then
        reaper.Main_SaveProjectEx(0, project_b, 0)
        check("project-saved-b", reaper.file_exists(project_b), project_b)
      end
      finish()
    end)
  end)()
end
