-- slot-automation and macro-automation (ENTH_SCENARIO = slot | macro): builds a project that automates host parameters
-- under a run of short notes and saves it for the xtask to render offline with `reaper -renderproject`; the xtask
-- then measures the audio.
--
--   slot:  slot 1 (the cutoff) ramps 0 -> 1 over 0-2 s, so the sound gets brighter; slot 64 (osc1's level) holds 0.5
--          until 2 s and ramps to 1 over 2-4 s, so it then gets louder.
--   macro: macro 1 (osc1's level) ramps 0 -> 1 over 0-2 s, so the sound gets louder; macro 8 (the cutoff) holds 0
--          until 2 s and ramps to 1 over 2-4 s, so it then gets brighter.
--
-- The two ramps never move at the same time, so loudness and brightness are measured apart. The notes are short and
-- repeated, on purpose: REAPER's offline render deactivates, reactivates and resets every CLAP plugin about 30 ms
-- after the render starts (measured) and does not resend held notes, so a held note goes silent for the rest of the
-- render. The next note-on makes sound again.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local tag = os.getenv("ENTH_TAG")
local scenario = os.getenv("ENTH_SCENARIO")
local MACRO_1, MACRO_8, SLOT_1, SLOT_64 = 0, 7, 8, 71
local PLANS = {
  slot = {
    { SLOT_1, { { 0, 0 }, { 2, 1 }, { 4, 1 } } },
    { SLOT_64, { { 0, 0.5 }, { 2, 0.5 }, { 4, 1 } } },
  },
  macro = {
    { MACRO_1, { { 0, 0 }, { 2, 1 }, { 4, 1 } } },
    { MACRO_8, { { 0, 0 }, { 2, 0 }, { 4, 1 } } },
  },
}

safe(function()
  local plan = assert(PLANS[scenario], "ENTH_SCENARIO must be slot or macro")
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  inject_state(track, fx)
  for _, lane in ipairs(plan) do
    local envelope = reaper.GetFXEnvelope(track, fx, lane[1], true)
    for _, point in ipairs(lane[2]) do
      reaper.InsertEnvelopePoint(envelope, point[1], point[2], 0, 0, false, true)
    end
    reaper.Envelope_SortPoints(envelope)
    check("envelope-points", reaper.CountEnvelopePoints(envelope) == #lane[2], "parameter " .. lane[1] .. ": " .. #lane[2] .. " points")
  end
  local item = reaper.CreateNewMIDIItemInProj(track, 0, 4.1, false)
  local take = reaper.GetActiveTake(item)
  local function ppq(seconds) return reaper.MIDI_GetPPQPosFromProjTime(take, seconds) end
  for i = 0, 31 do
    local start = i * 0.125
    reaper.MIDI_InsertNote(take, false, false, ppq(start), ppq(start + 0.11), 0, (i % 2 == 0) and 48 or 55, 100, true)
  end
  reaper.MIDI_Sort(take)

  reaper.GetSetProjectInfo_String(0, "RENDER_FILE", OUT, true)
  reaper.GetSetProjectInfo_String(0, "RENDER_PATTERN", "render-" .. scenario .. "-" .. tag, true)
  reaper.GetSetProjectInfo_String(0, "RENDER_FORMAT", "evaw", true)
  reaper.GetSetProjectInfo(0, "RENDER_BOUNDSFLAG", 1, true) -- entire project
  reaper.GetSetProjectInfo(0, "RENDER_SRATE", 48000, true)
  reaper.GetSetProjectInfo(0, "RENDER_CHANNELS", 2, true)
  local project = OUT .. "/" .. scenario .. "-automation-" .. tag .. ".rpp"
  reaper.Main_SaveProjectEx(0, project, 0)
  check("project-saved", reaper.file_exists(project), project)
  finish()
end)()
