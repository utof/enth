-- Must-haves (a) and (c), and (e) when run for the VST3 build:
--   create: load the plugin, check the 72 parameters and their ids, set two of them, add an
--           automation lane, save the project.
--   reload: in a NEW REAPER process, open that project and check everything came back.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local phase = os.getenv("ENTH_PHASE")
local tag = os.getenv("ENTH_TAG")
local project = OUT .. "/state-" .. tag .. ".rpp"

local MACRO_1, SLOT_1, SLOT_64 = 0, 8, 71
-- REAPER's ident is "<index>:<id>"; the id is nice-plug's 31-bit hash of the string id ("macro_1",
-- "slot_1", "slot_64"). If these change, saved projects lose their automation.
local GOLDEN = { [MACRO_1] = "0:825615486", [SLOT_1] = "8:1248030256", [SLOT_64] = "71:34232479" }

local function ident(track, fx, index)
  local _, id = reaper.TrackFX_GetParamIdent(track, fx, index)
  return id
end

local function ident_list(track, fx)
  local ids = {}
  for i = 0, 71 do ids[#ids + 1] = ident(track, fx, i) end
  return table.concat(ids, ",")
end

local function near(a, b) return math.abs(a - b) < 0.002 end

-- The names enth-state.patch.json gives (lane PL, the fork's runtime names): macro 1 is named, slots 1 and 64 drive
-- the "Tone" filter. REAPER reads VST3 titles live, so VST3 shows them at once; CLAP shows them after the rescan the
-- plugin asks for from its next process() call. Every other parameter keeps "Macro N" / "Slot N".
local RENAMED = { [MACRO_1] = "Bright", [SLOT_1] = "Tone · Cutoff", [SLOT_64] = "Tone · Resonance" }

if phase == "create" then
  safe(function()
    local track, fx = new_project_with_fx()
    if fx < 0 then return finish() end
    inject_state(track, fx)

    -- (a) 72 plugin parameters; REAPER appends its own Bypass, Wet and Delta after them. For VST3, nice-plug's
    -- `MidiConfig::MidiCCs` (the mod wheel; lane PL's decision in docs/plugin.md) adds a hidden, read-only parameter
    -- per mapped MIDI controller and channel after the 72, and REAPER lists them too. The plugin maps only CC 1
    -- (`Vst3Plugin::VST3_MIDI_CONTROLLERS`, the fork's patch 7): 16 parameters, where nice-plug's default is 2,080.
    local count = reaper.TrackFX_GetNumParams(track, fx)
    local midi_cc_params = tag == "vst3" and 16 or 0
    check("param-count", count == 75 + midi_cc_params,
      count .. " (72 plugin + " .. midi_cc_params .. " MIDI CC + REAPER's Bypass/Wet/Delta)")
    if tag == "vst3" then
      -- The mapped controllers, by name: "MIDI Ch. 1 CC 1" to "MIDI Ch. 16 CC 1", none automatable.
      local wrong, automatable, last = {}, 0, ""
      for i = 72, count - 4 do
        local _, name = reaper.TrackFX_GetParamName(track, fx, i)
        local want = "MIDI Ch. " .. (i - 71) .. " CC 1"
        if name ~= want then wrong[#wrong + 1] = i .. "=" .. tostring(name) end
        local _, auto = reaper.TrackFX_GetNamedConfigParm(track, fx, "param." .. i .. ".automatable")
        if auto == "1" then automatable = automatable + 1 end
        last = name
      end
      data("midi-cc-params", (count - 75) .. " after the 72, the last " .. last .. ", " .. automatable .. " automatable")
      check("midi-cc-params", #wrong == 0 and automatable == 0,
        #wrong == 0 and ("MIDI Ch. 1-16 CC 1, " .. automatable .. " automatable")
        or (table.concat(wrong, " ", 1, math.min(#wrong, 6)) .. (#wrong > 6 and " ..." or "")))
    end
    local bad_names, not_automatable = {}, 0
    for i = 0, 71 do
      local expected = i < 8 and ("Macro " .. (i + 1)) or ("Slot " .. (i - 7))
      local _, name = reaper.TrackFX_GetParamName(track, fx, i)
      if name ~= expected and name ~= RENAMED[i] then bad_names[#bad_names + 1] = i .. "=" .. tostring(name) end
      local _, automatable = reaper.TrackFX_GetNamedConfigParm(track, fx, "param." .. i .. ".automatable")
      if automatable ~= "1" then not_automatable = not_automatable + 1 end
    end
    check("param-names", #bad_names == 0, #bad_names == 0 and "Macro 1-8, Slot 1-64 (or the patch's names)" or table.concat(bad_names, " "))
    check("param-automatable", not_automatable == 0, not_automatable .. " not automatable")
    local ids = ident_list(track, fx)
    data("idents-create", ids)
    local golden_bad = {}
    for index, expected in pairs(GOLDEN) do
      if ident(track, fx, index) ~= expected then
        golden_bad[#golden_bad + 1] = index .. ": " .. tostring(ident(track, fx, index)) .. " != " .. expected
      end
    end
    check("param-ids-stable", #golden_bad == 0, #golden_bad == 0 and "macro_1, slot_1, slot_64 match" or table.concat(golden_bad, "; "))

    -- (a) automate a slot; (c) change two parameters.
    local envelope = reaper.GetFXEnvelope(track, fx, SLOT_64, true)
    reaper.InsertEnvelopePoint(envelope, 0.0, 0.25, 0, 0, false, true)
    reaper.InsertEnvelopePoint(envelope, 2.0, 0.75, 0, 0, false, true)
    reaper.Envelope_SortPoints(envelope)
    reaper.TrackFX_SetParamNormalized(track, fx, MACRO_1, 0.7)
    reaper.TrackFX_SetParamNormalized(track, fx, SLOT_1, 0.3)

    -- VST3 only applies a host-set value on the next process() call, so wait before reading it back.
    wait(1.5, function()
      local m, s = norm(track, fx, MACRO_1), norm(track, fx, SLOT_1)
      check("set-values", near(m, 0.7) and near(s, 0.3), string.format("macro 1 = %.4f, slot 1 = %.4f", m, s))
      local unrenamed = {}
      for index, want in pairs(RENAMED) do
        local _, name = reaper.TrackFX_GetParamName(track, fx, index)
        if name ~= want then unrenamed[#unrenamed + 1] = index .. "=" .. tostring(name) end
      end
      check("param-renamed", #unrenamed == 0, #unrenamed == 0 and "Bright, Tone · Cutoff, Tone · Resonance" or table.concat(unrenamed, " "))
      write_file("chunk-" .. tag .. "-create.b64", state_chunk(track, fx))
      reaper.Main_SaveProjectEx(0, project, 0)
      check("project-saved", reaper.file_exists(project), project)
      finish()
    end)
  end)()
else
  safe(function()
    reaper.Main_openProject("noprompt:" .. project)
    local track = reaper.GetTrack(0, 0)
    wait(1.5, function()
      check("reload-fx", track ~= nil and reaper.TrackFX_GetCount(track) == 1, "one plugin on track 1")
      if not track then return finish() end
      local m, s = norm(track, 0, MACRO_1), norm(track, 0, SLOT_1)
      check("reload-values", near(m, 0.7) and near(s, 0.3), string.format("macro 1 = %.4f, slot 1 = %.4f", m, s))
      data("idents-reload", ident_list(track, 0))
      local envelope = reaper.GetFXEnvelope(track, 0, SLOT_64, false)
      local points = envelope and reaper.CountEnvelopePoints(envelope) or -1
      check("reload-automation", points == 2, "slot 64 lane has " .. points .. " points")
      write_file("chunk-" .. tag .. "-reload.b64", state_chunk(track, 0))
      finish()
    end)
  end)()
end
