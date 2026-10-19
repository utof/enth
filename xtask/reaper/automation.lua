-- Must-have (a), audible half: automate Macro 1 (gain) and Slot 1 (cutoff) from 0 to 1 over two
-- seconds under a run of short notes, and save a project that the xtask renders offline with
-- `reaper -renderproject`. The xtask then checks that the audio gets louder and brighter.
--
-- Short, repeated notes instead of held ones, on purpose: REAPER's offline render deactivates,
-- reactivates and resets every CLAP plugin about 30 ms after the render starts (measured, also
-- with the researcher's spike-np) and does not resend held notes, so a held note goes silent for
-- the rest of the render. The next note-on makes sound again.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local tag = os.getenv("ENTH_TAG")
local MACRO_1, SLOT_1 = 0, 8

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  for _, index in ipairs({ MACRO_1, SLOT_1 }) do
    local envelope = reaper.GetFXEnvelope(track, fx, index, true)
    reaper.InsertEnvelopePoint(envelope, 0.0, 0.0, 0, 0, false, true)
    reaper.InsertEnvelopePoint(envelope, 2.0, 1.0, 0, 0, false, true)
    reaper.Envelope_SortPoints(envelope)
  end
  local item = reaper.CreateNewMIDIItemInProj(track, 0, 2.5, false)
  local take = reaper.GetActiveTake(item)
  local function ppq(seconds) return reaper.MIDI_GetPPQPosFromProjTime(take, seconds) end
  for i = 0, 19 do
    local start = i * 0.125
    reaper.MIDI_InsertNote(take, false, false, ppq(start), ppq(start + 0.11), 0, (i % 2 == 0) and 48 or 55, 100, true)
  end
  reaper.MIDI_Sort(take)

  reaper.GetSetProjectInfo_String(0, "RENDER_FILE", OUT, true)
  reaper.GetSetProjectInfo_String(0, "RENDER_PATTERN", "render-" .. tag, true)
  reaper.GetSetProjectInfo_String(0, "RENDER_FORMAT", "evaw", true)
  reaper.GetSetProjectInfo(0, "RENDER_BOUNDSFLAG", 1, true) -- entire project
  reaper.GetSetProjectInfo(0, "RENDER_SRATE", 48000, true)
  reaper.GetSetProjectInfo(0, "RENDER_CHANNELS", 2, true)
  local project = OUT .. "/auto-" .. tag .. ".rpp"
  reaper.Main_SaveProjectEx(0, project, 0)
  check("project-saved", reaper.file_exists(project), project)
  finish()
end)()
