-- The driver of `cargo xtask reaper-soak` (lane SOAK): N enth instances in REAPER for ENTH_SOAK_SECS seconds of wall
-- clock, under Dummy Audio and Xvfb, played and edited the way a long session would:
--   * notes through REAPER's virtual keyboard (the transport does not run under Dummy Audio; research 04): one track per
--     instance, each on its own MIDI channel, four patterns (chords, a fast run, bursts of 24 notes that steal
--     voices, long held notes with the mod wheel, pitch bend and sustain pedal), always released again;
--   * parameter sweeps: a random walk over the 72 host parameters of every instance (the automation stand-in);
--   * structural edits: a state load every ENTH_SOAK_LOAD_SECS (the next of the xtask's patch chunks, round robin), and
--     every ENTH_SOAK_CHURN_SECS an instance deleted and added again;
--   * the editor opened every ENTH_SOAK_EDITOR_SECS (round robin over the instances), drawn, edited with XTest events
--     (xinput.py), held, closed; the process's memory read before and after each cycle (smaps_rollup).
-- Settings come from the environment (the xtask sets them; see soak.rs). Results are lines in the log (common.lua):
--   data begin|beat|peaks|mem|cycle|event|summary <key=value ...>, check <name> ok|FAIL <detail>, data finished 1.
-- The xtask samples the REAPER process from outside as well (PSS, threads, descriptors), watches this log for a
-- heartbeat (a hang) and for the process dying (a crash, or an allocation abort of the debug build).
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local function num(name, default)
  return tonumber(os.getenv(name) or "") or default
end
local SECS = num("ENTH_SOAK_SECS", 60)
local N = math.max(1, math.floor(num("ENTH_SOAK_INSTANCES", 4)))
local EDITOR_PERIOD = num("ENTH_SOAK_EDITOR_SECS", 60) -- 0: the editor is never opened
local EDITOR_HOLD = num("ENTH_SOAK_EDITOR_HOLD", 5)
local NOTES = num("ENTH_SOAK_NOTES", 1) == 1
local SWEEPS = num("ENTH_SOAK_SWEEPS", 1) == 1
local LOAD_PERIOD = num("ENTH_SOAK_LOAD_SECS", 90) -- 0: no state loads
local CHURN_PERIOD = num("ENTH_SOAK_CHURN_SECS", 300) -- 0: no instance churn
local CHURN_WITH_EDITOR = num("ENTH_SOAK_CHURN_EDITOR", 1) == 1 -- every 4th churn deletes an instance whose editor is open
local BEAT = num("ENTH_SOAK_BEAT_SECS", 2)
local SETTLE = num("ENTH_SOAK_SETTLE_SECS", 3) -- after a close, before the memory is read
local SWEEP_STEP = num("ENTH_SOAK_SWEEP_STEP", 0.05)
local INPUT_DEVICE = num("ENTH_SOAK_INPUT_DEVICE", 63) -- 63: all MIDI inputs (the virtual keyboard is among them)
local CONTROL = os.getenv("ENTH_SOAK_CONTROL_FX") or ""
local IS_CONTROL = CONTROL ~= ""
local FX_NAME = IS_CONTROL and CONTROL or os.getenv("ENTH_FX")
local NPARAMS = 72 -- enth's host parameters; a control effect has whatever it has

-- ---- the deterministic random source (a 64-bit LCG; Lua integers wrap) --------------------------------------------
local rng = math.floor(num("ENTH_SOAK_SEED", 20261020))
local function rnd()
  rng = rng * 6364136223846793005 + 1442695040888963407
  return ((rng >> 33) & 0x7FFFFFFF) / 2147483648.0
end
local function rint(lo, hi) return lo + math.floor(rnd() * (hi - lo + 1)) end

-- ---- the xtask's patch chunks (base64, the files named by ENTH_SOAK_CHUNKS, comma separated) -----------------------
local CHUNKS = {}
for path in (os.getenv("ENTH_SOAK_CHUNKS") or ""):gmatch("[^,]+") do
  local f = io.open(path, "r")
  if f then
    CHUNKS[#CHUNKS + 1] = { name = path:match("chunk%-%a+%-(.-)%-in%.b64$") or path, text = f:read("*a") }
    f:close()
  end
end
local CHUNK_KEY = os.getenv("ENTH_TAG") == "clap" and "clap_chunk" or "vst_chunk"

-- ---- process memory -----------------------------------------------------------------------------------------------
local function read_mem()
  local f = io.open("/proc/self/smaps_rollup", "r")
  if not f then return { pss = 0, anon = 0, swap = 0, rss = 0 } end
  local t = f:read("*a")
  f:close()
  local function kb(name) return tonumber(t:match("\n" .. name .. ":%s+(%d+) kB")) or 0 end
  return { pss = kb("Pss"), anon = kb("Pss_Anon"), swap = kb("SwapPss"), rss = kb("Rss") }
end

local function kv(t)
  local keys = {}
  for k in pairs(t) do keys[#keys + 1] = k end
  table.sort(keys)
  local parts = {}
  for _, k in ipairs(keys) do parts[#parts + 1] = k .. "=" .. tostring(t[k]) end
  return table.concat(parts, " ")
end

local function event(kind, detail) data("event", kind .. " " .. (detail or "")) end

-- ---- state ----------------------------------------------------------------------------------------------------------
local tracks, fxs, held = {}, {}, {}
local counts = { note_on = 0, note_off = 0, cc = 0, sets = 0, loads = 0, load_fail = 0, churns = 0, churn_fail = 0,
  churn_open = 0, cycles = 0, cycles_drew = 0, cycles_closed = 0, edits = 0, edits_applied = 0, silent_minutes = 0,
  param_fail = 0 }
local editor_open, editor_busy, editor_inst = false, false, 0
local editor_changed = reaper.time_precise() -- when the editor last opened or closed
local peak_win = {}
local T0

-- The editor's state, written to the log at every change so the xtask can tell a settled, editor-closed memory
-- sample from one taken in the middle of a cycle: `busy` is a whole cycle (open, edit, hold, close, settle).
local function set_editor(busy, open)
  editor_busy, editor_open = busy, open
  editor_changed = reaper.time_precise()
  if T0 then
    data("mark", kv({ t = string.format("%.2f", editor_changed - T0), busy = busy and 1 or 0, editor = open and 1 or 0 }))
  end
end

local function window_title(i) return FX_NAME .. " - Track " .. i end

-- ---- notes ----------------------------------------------------------------------------------------------------------
local queue = {}
local function at(due, fn)
  local n = #queue + 1
  queue[n] = { due, fn }
  while n > 1 and queue[n - 1][1] > queue[n][1] do
    queue[n - 1], queue[n] = queue[n], queue[n - 1]
    n = n - 1
  end
end

local function note_on(i, pitch, vel)
  reaper.StuffMIDIMessage(0, 0x90 | (i - 1), pitch, vel)
  held[i][pitch] = true
  counts.note_on = counts.note_on + 1
end
local function note_off(i, pitch)
  reaper.StuffMIDIMessage(0, 0x80 | (i - 1), pitch, 0)
  held[i][pitch] = nil
  counts.note_off = counts.note_off + 1
end
local function cc(i, number, value)
  reaper.StuffMIDIMessage(0, 0xB0 | (i - 1), number, value)
  counts.cc = counts.cc + 1
end
local function bend(i, value) -- 0..16383, 8192 is centre
  reaper.StuffMIDIMessage(0, 0xE0 | (i - 1), value & 127, (value >> 7) & 127)
  counts.cc = counts.cc + 1
end
local function release_all(i)
  local pitches = {}
  for p in pairs(held[i]) do pitches[#pitches + 1] = p end
  for _, p in ipairs(pitches) do note_off(i, p) end
end

local SCALE = { 0, 3, 5, 7, 10 }
local ROOTS = { 48, 53, 55, 50 }

-- Pattern 1: a four-note chord every 0.5 s, held 0.4 s, the root changing every four chords.
local function pattern_chords(i, k)
  local root = ROOTS[((k // 4) % #ROOTS) + 1]
  for _, iv in ipairs({ 0, 7, 12, 16 }) do note_on(i, root + iv, 90 + rint(0, 30)) end
  at(reaper.time_precise() + 0.4, function()
    for _, iv in ipairs({ 0, 7, 12, 16 }) do if held[i][root + iv] then note_off(i, root + iv) end end
  end)
  at(reaper.time_precise() + 0.5, function() pattern_chords(i, k + 1) end)
end

-- Pattern 2: a fast run of single notes (a note every 0.12 s, 0.1 s long) over a pentatonic scale.
local function pattern_run(i, k)
  local pitch = 48 + 12 * rint(0, 3) + SCALE[rint(1, #SCALE)]
  note_on(i, pitch, rint(60, 127))
  at(reaper.time_precise() + 0.1, function() if held[i][pitch] then note_off(i, pitch) end end)
  at(reaper.time_precise() + 0.12, function() pattern_run(i, k + 1) end)
end

-- Pattern 3: every 12 s a burst of 24 notes in 0.5 s (more than the 16 voices, so voices are stolen), held 3 s.
local function pattern_burst(i, k)
  local base = reaper.time_precise()
  local pitches = {}
  for n = 0, 23 do
    local pitch = 36 + ((n * 7 + k * 3) % 60)
    pitches[#pitches + 1] = pitch
    at(base + n * 0.02, function() if not held[i][pitch] then note_on(i, pitch, 70 + (n * 3) % 50) end end)
  end
  at(base + 3.5, function() for _, p in ipairs(pitches) do if held[i][p] then note_off(i, p) end end end)
  at(base + 12, function() pattern_burst(i, k + 1) end)
end

-- Pattern 4: three held notes for 6 s with the mod wheel, pitch bend and the sustain pedal moving; every 7th round
-- ends with "all notes off" (CC 123) instead of note offs; then 2 s of silence.
local function pattern_expressive(i, k)
  local base = reaper.time_precise()
  local pitches = { 45 + (k % 5), 52 + (k % 5), 57 + (k % 5) }
  for _, p in ipairs(pitches) do note_on(i, p, 100) end
  for step = 0, 59 do -- ten controller updates a second
    at(base + step * 0.1, function()
      cc(i, 1, math.floor(63.5 + 63.5 * math.sin(step * 0.3)))
      bend(i, math.floor(8192 + 3000 * math.sin(step * 0.5)))
    end)
  end
  at(base + 2, function() cc(i, 64, 127) end)
  at(base + 4, function() cc(i, 64, 0) end)
  at(base + 6, function()
    if k % 7 == 6 then
      cc(i, 123, 0)
      held[i] = {}
    else
      for _, p in ipairs(pitches) do if held[i][p] then note_off(i, p) end end
    end
    bend(i, 8192)
    cc(i, 1, 0)
  end)
  at(base + 8, function() pattern_expressive(i, k + 1) end)
end

local PATTERNS = { pattern_chords, pattern_run, pattern_burst, pattern_expressive }

-- ---- parameter sweeps -----------------------------------------------------------------------------------------------
local walk = {} -- walk[i][p] = { cur, target }
local function sweep(now)
  for i = 1, N do
    for _ = 1, 2 do
      local p = (rnd() < 0.5) and rint(0, 7) or rint(0, NPARAMS - 1)
      local w = walk[i][p]
      if not w then w = { rnd(), rnd() }; walk[i][p] = w end
      local d = w[2] - w[1]
      if math.abs(d) < 0.02 then
        w[2] = (rnd() < 0.1) and (rnd() < 0.5 and 0 or 1) or rnd() -- now and then straight to an extreme
      else
        w[1] = w[1] + d * 0.35
      end
      if not reaper.TrackFX_SetParamNormalized(tracks[i], fxs[i], p, w[1]) then counts.param_fail = counts.param_fail + 1 end
      counts.sets = counts.sets + 1
    end
  end
end

-- ---- structural edits ------------------------------------------------------------------------------------------------
local next_chunk, next_load_inst, next_churn_inst = 1, 1, 1

local function load_chunk(i, label)
  if IS_CONTROL or #CHUNKS == 0 then return end
  local chunk = CHUNKS[next_chunk]
  next_chunk = next_chunk % #CHUNKS + 1
  local ok = reaper.TrackFX_SetNamedConfigParm(tracks[i], fxs[i], CHUNK_KEY, chunk.text)
  counts.loads = counts.loads + 1
  if not ok then counts.load_fail = counts.load_fail + 1 end
  event(label or "state-load", string.format("inst=%d chunk=%s ok=%d", i, chunk.name, ok and 1 or 0))
end

local function churn(i)
  local ok_delete = reaper.TrackFX_Delete(tracks[i], fxs[i])
  local fx = reaper.TrackFX_AddByName(tracks[i], FX_NAME, false, -1)
  counts.churns = counts.churns + 1
  if fx < 0 or not ok_delete then
    counts.churn_fail = counts.churn_fail + 1
    event("churn-failed", string.format("inst=%d deleted=%s fx=%d", i, tostring(ok_delete), fx))
    fxs[i] = fx < 0 and 0 or fx
    return
  end
  fxs[i] = fx
  walk[i] = {}
  event("churn", string.format("inst=%d fx=%d", i, fx))
  load_chunk(i, "churn-load")
end

-- ---- the editor -------------------------------------------------------------------------------------------------------
local function window_geometry(title)
  local ok, text = xinput("client " .. string.format("%q", title))
  if not ok then return nil end
  local x, y, w, h = text:match("^%d+ (%-?%d+) (%-?%d+) (%d+) (%d+)$")
  if not x then return nil end
  return tonumber(x), tonumber(y), tonumber(w), tonumber(h)
end

local function window_viewable(title)
  local ok, text = xinput("client " .. string.format("%q", title))
  return ok and text ~= ""
end

-- One editor cycle on instance i: open, wait for the first drawn frame, edit a knob with real X events, hold, close,
-- wait for the window to go, let the memory settle and read it. `fn` is called at the end.
local EDIT_POINT = { 366, 178 } -- osc1's Level knob in the editor's "lanes" layout (dirty.lua)
local function editor_cycle(i, fn)
  set_editor(true, false)
  editor_inst = i
  counts.cycles = counts.cycles + 1
  local n = counts.cycles
  local tr, fx = tracks[i], fxs[i]
  local before = read_mem()
  local title = window_title(i)
  local t_open = reaper.time_precise()
  local row = { n = n, inst = i, before_pss = before.pss, before_anon = before.anon, before_swap = before.swap,
    before_rss = before.rss }
  local function finish_cycle()
    set_editor(true, false)
    wait(SETTLE, function()
      local after = read_mem()
      row.after_pss, row.after_anon, row.after_swap, row.after_rss = after.pss, after.anon, after.swap, after.rss
      row.t = string.format("%.1f", reaper.time_precise() - T0)
      data("cycle", kv(row))
      set_editor(false, false)
      fn()
    end)
  end
  local function close()
    hide_editor(tr, fx)
    if IS_CONTROL then
      wait_until(function() return reaper.TrackFX_GetFloatingWindow(tr, fx) == nil end, 5, 0.1, function(gone)
        row.gone = gone and 1 or 0
        if gone then counts.cycles_closed = counts.cycles_closed + 1 end
        finish_cycle()
      end)
    else
      wait_until(function() return not window_viewable(title) end, 5, 0.2, function(gone)
        row.gone = gone and 1 or 0
        if gone then counts.cycles_closed = counts.cycles_closed + 1 end
        finish_cycle()
      end)
    end
  end
  local function opened(drew, seconds)
    set_editor(true, true)
    row.drew, row.first_frame = drew and 1 or 0, string.format("%.2f", seconds)
    if drew then counts.cycles_drew = counts.cycles_drew + 1 end
    wait(EDITOR_HOLD / 2, function()
      if not IS_CONTROL and drew then
        local x, y, w, h = window_geometry(title)
        if x then
          local before_state = state_chunk(tr, fx)
          local dy = (n % 2 == 0) and 30 or -30
          xinput(string.format("drag %d %d %d %d", x + EDIT_POINT[1], y + EDIT_POINT[2], x + EDIT_POINT[1],
            y + EDIT_POINT[2] + dy))
          counts.edits = counts.edits + 1
          wait(0.3, function()
            if state_chunk(tr, fx) ~= before_state then counts.edits_applied = counts.edits_applied + 1 end
          end)
        end
      end
      wait(EDITOR_HOLD / 2, close)
    end)
  end
  if IS_CONTROL then
    reaper.TrackFX_Show(tr, fx, 3)
    wait_until(function() return reaper.TrackFX_GetFloatingWindow(tr, fx) ~= nil end, 5, 0.1, function(shown)
      opened(shown, reaper.time_precise() - t_open)
    end)
  else
    show_editor(tr, fx, opened, title)
  end
end

-- ---- the run ----------------------------------------------------------------------------------------------------------
local function setup()
  reaper.Main_OnCommand(40023, 0) -- File: New project
  for i = 1, N do
    reaper.InsertTrackAtIndex(i - 1, true)
    local tr = reaper.GetTrack(0, i - 1)
    tracks[i] = tr
    held[i] = {}
    walk[i] = {}
    peak_win[i] = 0
    fxs[i] = reaper.TrackFX_AddByName(tr, FX_NAME, false, -1)
    check("load-" .. i, fxs[i] >= 0, FX_NAME)
    if fxs[i] < 0 then return false end
    reaper.SetMediaTrackInfo_Value(tr, "I_PERFFLAGS", 2) -- no anticipative FX: the plugin runs in the audio thread
    reaper.SetMediaTrackInfo_Value(tr, "I_RECINPUT", 4096 + (INPUT_DEVICE << 5) + i) -- MIDI, channel i
    reaper.SetMediaTrackInfo_Value(tr, "I_RECMON", 1)
    reaper.SetMediaTrackInfo_Value(tr, "I_RECARM", 1)
  end
  -- Start every instance on a different patch, so the first state load is a change.
  for i = 1, N do load_chunk(i, "initial-load") end
  return true
end

local function run()
  if not setup() then return finish() end
  local nparams = reaper.TrackFX_GetNumParams(tracks[1], fxs[1])
  if IS_CONTROL then NPARAMS = math.max(1, nparams) end
  local count0 = counts.loads
  if reaper.Audio_IsRunning() == 0 then reaper.Audio_Init() end
  T0 = reaper.time_precise()
  local m0 = read_mem()
  data("begin", kv({ instances = N, fx = "'" .. FX_NAME .. "'", params = nparams, audio = reaper.Audio_IsRunning(),
    chunks = #CHUNKS, secs = SECS, pss0 = m0.pss, anon0 = m0.anon, notes = NOTES and 1 or 0,
    sweeps = SWEEPS and 1 or 0, editor_period = EDITOR_PERIOD, load_period = LOAD_PERIOD,
    churn_period = CHURN_PERIOD, control = IS_CONTROL and 1 or 0 }))
  local a0 = reaper.GetUnderrunTime()

  if NOTES then
    for i = 1, N do
      -- Spread the instances' first notes so they do not all start on the same tick.
      at(T0 + 0.5 + 0.1 * i, function() PATTERNS[((i - 1) % #PATTERNS) + 1](i, 0) end)
    end
  end

  local next_sweep, next_beat, next_mem = T0 + 1, T0, T0 + 30
  local next_editor = EDITOR_PERIOD > 0 and (T0 + math.min(EDITOR_PERIOD, 10)) or math.huge
  local next_load = LOAD_PERIOD > 0 and (T0 + LOAD_PERIOD) or math.huge
  local next_churn = CHURN_PERIOD > 0 and (T0 + CHURN_PERIOD) or math.huge
  local minute = 0
  local last_minute_at = T0
  local ticks, last_underrun = 0, a0
  local xrun_events = 0
  local finishing = false

  local function finish_run()
    if finishing then return end
    finishing = true
    for i = 1, N do
      release_all(i)
      cc(i, 123, 0) -- all notes off, in case a note the script lost track of is still sounding
      cc(i, 64, 0)
    end
    local m1 = read_mem()
    do -- the last, partial minute of peaks
      local row = { minute = minute + 1, final = 1 }
      for i = 1, N do row["p" .. i] = string.format("%.4f", peak_win[i]) end
      data("peaks", kv(row))
    end
    local a1, m_under = reaper.GetUnderrunTime()
    data("summary", kv({ elapsed = string.format("%.1f", reaper.time_precise() - T0), note_on = counts.note_on,
      note_off = counts.note_off, cc = counts.cc, sets = counts.sets, param_fail = counts.param_fail,
      loads = counts.loads, load_fail = counts.load_fail, churns = counts.churns, churn_fail = counts.churn_fail,
      cycles = counts.cycles, cycles_drew = counts.cycles_drew, cycles_closed = counts.cycles_closed,
      edits = counts.edits, edits_applied = counts.edits_applied, xrun_events = xrun_events,
      underrun_audio = a1, underrun_media = m_under, ticks = ticks, pss_end = m1.pss, anon_end = m1.anon }))
    wait(0.5, function()
      for i = 1, N do hide_editor(tracks[i], fxs[i]) end
      wait(0.5, finish)
    end)
  end

  local function tick()
    local now = reaper.time_precise()
    ticks = ticks + 1
    if now - T0 >= SECS and not editor_busy then return finish_run() end
    if finishing then return end
    -- Due notes and controller moves.
    while queue[1] and queue[1][1] <= now do
      local ev = table.remove(queue, 1)
      ev[2]()
    end
    if SWEEPS and now >= next_sweep then
      sweep(now)
      next_sweep = now + SWEEP_STEP
    end
    if NOTES then
      for i = 1, N do
        local pk = reaper.Track_GetPeakInfo(tracks[i], 0)
        if pk > peak_win[i] then peak_win[i] = pk end
      end
    end
    local a = reaper.GetUnderrunTime()
    if a ~= last_underrun then xrun_events = xrun_events + 1; last_underrun = a end
    if now >= next_load and not finishing then
      next_load = now + LOAD_PERIOD
      load_chunk(next_load_inst)
      next_load_inst = next_load_inst % N + 1
    end
    if now >= next_churn and not editor_busy then
      next_churn = now + CHURN_PERIOD
      counts.churn_round = (counts.churn_round or 0) + 1
      local i = next_churn_inst
      next_churn_inst = next_churn_inst % N + 1
      if CHURN_WITH_EDITOR and not IS_CONTROL and counts.churn_round % 4 == 0 then
        -- The user deletes the plugin with its editor open: the window must go with it, and nothing may crash.
        set_editor(true, false)
        editor_inst = i
        show_editor(tracks[i], fxs[i], function(drew)
          set_editor(true, true)
          wait(1.0, function()
            counts.churn_open = counts.churn_open + 1
            churn(i)
            set_editor(true, false)
            wait(1.0, function()
              local still = window_viewable(window_title(i))
              event("churn-with-editor", string.format("inst=%d drew=%d window_left=%d", i, drew and 1 or 0, still and 1 or 0))
              if still then hide_editor(tracks[i], fxs[i]) end
              set_editor(false, false)
            end)
          end)
        end, window_title(i))
      else
        churn(i)
      end
    end
    if now >= next_editor and not editor_busy then
      next_editor = now + EDITOR_PERIOD
      editor_cycle(((counts.cycles) % N) + 1, function() end)
    end
    if now >= next_mem then
      next_mem = now + 30
      local m = read_mem()
      data("mem", kv({ t = string.format("%.1f", now - T0), pss = m.pss, anon = m.anon, swap = m.swap, rss = m.rss,
        editor = editor_open and 1 or 0 }))
    end
    if now >= next_beat then
      next_beat = now + BEAT
      data("beat", kv({ t = string.format("%.1f", now - T0), editor = editor_open and 1 or 0, busy = editor_busy and 1 or 0,
        since = string.format("%.1f", now - editor_changed), notes = counts.note_on, sets = counts.sets,
        loads = counts.loads, churns = counts.churns, cycles = counts.cycles }))
    end
    if now - last_minute_at >= 60 then
      last_minute_at = now
      minute = minute + 1
      local row = { minute = minute }
      local any = false
      for i = 1, N do
        row["p" .. i] = string.format("%.4f", peak_win[i])
        if peak_win[i] > 1e-6 then any = true end
        peak_win[i] = 0
      end
      if NOTES and not any then counts.silent_minutes = counts.silent_minutes + 1 end
      data("peaks", kv(row))
    end
    reaper.defer(safe(tick))
  end
  reaper.defer(safe(tick))
end

safe(run)()
