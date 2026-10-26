-- wt (lane SKS, F2m): a patch with a wavetable oscillator (`wt`, kind 8) phase-modulated by another generator through an
-- audio route, loaded as state, played by short notes, saved, reloaded in a new process and played again.
--   create:  load the patch as state, check the host's names for its two slots, dump the saved state, build the notes (a MIDI item) and save the project (a).
--            The xtask renders (a) offline: the notes' level and brightness are measured against what the engine renders.
--   reload:  new process: open (a), dump the saved state, name its render differently and save the project as (b).
--            The xtask renders (b) too: the reloaded project must sound the same.
-- The notes are the ones `render.lua` plays (keys 48 and 55 alternating, 0.11 s every 0.125 s, velocity 100), and short
-- and repeated for the same reason (REAPER's offline render restarts a CLAP plugin about 30 ms in).
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local phase = os.getenv("ENTH_PHASE")
local tag = os.getenv("ENTH_TAG")
local project_a = OUT .. "/wt-" .. tag .. "-a.rpp"
local project_b = OUT .. "/wt-" .. tag .. "-b.rpp"

-- The host lists a wavetable parameter on a slot by name, as it lists any other (slot 1 is host parameter 8).
local SLOT_1, SLOT_2 = 8, 9
local function slot_names(label, track, fx)
  local _, one = reaper.TrackFX_GetParamName(track, fx, SLOT_1)
  local _, two = reaper.TrackFX_GetParamName(track, fx, SLOT_2)
  check("wt-slots-named-" .. label, one == "Wavetable 1 · PM index" and two == "Wavetable 1 · Position",
    "slot 1 = " .. tostring(one) .. ", slot 2 = " .. tostring(two))
end

local function render_to(name)
  reaper.GetSetProjectInfo_String(0, "RENDER_FILE", OUT, true)
  reaper.GetSetProjectInfo_String(0, "RENDER_PATTERN", name, true)
  reaper.GetSetProjectInfo_String(0, "RENDER_FORMAT", "evaw", true)
  reaper.GetSetProjectInfo(0, "RENDER_BOUNDSFLAG", 1, true) -- entire project
  reaper.GetSetProjectInfo(0, "RENDER_SRATE", 48000, true)
  reaper.GetSetProjectInfo(0, "RENDER_CHANNELS", 2, true)
end

if phase == "create" then
  safe(function()
    local track, fx = new_project_with_fx()
    if fx < 0 then return finish() end
    inject_state(track, fx)
    wait(1.5, function()
      slot_names("create", track, fx)
      dump_chunk("wt-create", track, fx)
      local item = reaper.CreateNewMIDIItemInProj(track, 0, 4.1, false)
      local take = reaper.GetActiveTake(item)
      local function ppq(seconds) return reaper.MIDI_GetPPQPosFromProjTime(take, seconds) end
      for i = 0, 31 do
        local start = i * 0.125
        reaper.MIDI_InsertNote(take, false, false, ppq(start), ppq(start + 0.11), 0, (i % 2 == 0) and 48 or 55, 100, true)
      end
      reaper.MIDI_Sort(take)
      local _, notes = reaper.MIDI_CountEvts(take)
      check("notes-built", notes == 32, notes .. " notes in the MIDI item")
      render_to("render-wt-" .. tag)
      reaper.Main_SaveProjectEx(0, project_a, 0)
      check("project-saved-a", reaper.file_exists(project_a), project_a)
      finish()
    end)
  end)()
else
  safe(function()
    reaper.Main_openProject("noprompt:" .. project_a)
    local track = reaper.GetTrack(0, 0)
    wait(1.5, function()
      check("reload-fx", track ~= nil and reaper.TrackFX_GetCount(track) == 1, "one plugin on track 1")
      if not track then return finish() end
      slot_names("reload", track, 0)
      dump_chunk("wt-reload", track, 0)
      render_to("render-wt-" .. tag .. "-reload")
      reaper.Main_SaveProjectEx(0, project_b, 0)
      check("project-saved-b", reaper.file_exists(project_b), project_b)
      finish()
    end)
  end)()
end
