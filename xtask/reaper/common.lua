-- Shared helpers for the headless REAPER tests run by `cargo xtask reaper-test`.
-- Settings come from environment variables set by the xtask:
--   ENTH_OUT (results dir), ENTH_EMPTY_PROJECT, ENTH_LOG (log file name inside ENTH_OUT),
--   ENTH_FX (REAPER plugin name, e.g. "CLAPi: enth (enth)"), ENTH_TAG (e.g. "clap"), ENTH_PHASE.
-- Every result is a line in the log: `check<TAB>name<TAB>ok|FAIL<TAB>detail` or `data<TAB>key<TAB>value`.

OUT = os.getenv("ENTH_OUT")
local log_file = assert(io.open(OUT .. "/" .. os.getenv("ENTH_LOG"), "a"))

function check(name, ok, detail)
  log_file:write(string.format("check\t%s\t%s\t%s\n", name, ok and "ok" or "FAIL", tostring(detail or "")))
  log_file:flush()
end

function data(key, value)
  log_file:write(string.format("data\t%s\t%s\n", key, tostring(value)))
  log_file:flush()
end

-- Quit without REAPER asking to save: open an empty project first, then run "File: Quit".
function finish()
  data("finished", "1")
  log_file:close()
  reaper.Main_openProject("noprompt:" .. os.getenv("ENTH_EMPTY_PROJECT"))
  reaper.Main_OnCommand(40004, 0)
end

-- Runs fn; on a Lua error records a failed check and quits instead of leaving a modal error dialog.
function safe(fn)
  return function(...)
    local ok, err = xpcall(fn, debug.traceback, ...)
    if not ok then
      check("script-error", false, (tostring(err):gsub("[\r\n\t]+", " | ")))
      finish()
    end
  end
end

-- Calls fn after sec seconds, letting REAPER keep running (its audio thread processes meanwhile).
function wait(sec, fn)
  local t0 = reaper.time_precise()
  local function poll()
    if reaper.time_precise() - t0 < sec then reaper.defer(poll) else safe(fn)() end
  end
  reaper.defer(poll)
end

function new_project_with_fx()
  reaper.Main_OnCommand(40023, 0) -- File: New project
  reaper.InsertTrackAtIndex(0, true)
  local track = reaper.GetTrack(0, 0)
  local fx = reaper.TrackFX_AddByName(track, os.getenv("ENTH_FX"), false, -1)
  check("load", fx >= 0, os.getenv("ENTH_FX"))
  return track, fx
end

function write_file(name, text)
  local f = assert(io.open(OUT .. "/" .. name, "w"))
  f:write(text)
  f:close()
end

-- The plugin state as base64 text; REAPER names the setting after the format.
function state_chunk(track, fx)
  for _, key in ipairs({ "clap_chunk", "vst_chunk" }) do
    local ok, chunk = reaper.TrackFX_GetNamedConfigParm(track, fx, key)
    if ok then return chunk end
  end
  return ""
end

function norm(track, fx, index)
  return reaper.TrackFX_GetParamNormalized(track, fx, index)
end

-- Loads a plugin state (base64, as REAPER's `clap_chunk` / `vst_chunk` hold it) from the file named by the environment
-- variable `var` (default ENTH_STATE_CHUNK_FILE), if the xtask gave one: the product plugin's patch for a scenario
-- (P1b lane SK). Nothing happens without it.
function inject_state(track, fx, var)
  local path = os.getenv(var or "ENTH_STATE_CHUNK_FILE")
  if not path or path == "" then return end
  local f = assert(io.open(path, "r"))
  local chunk = f:read("*a")
  f:close()
  local key = os.getenv("ENTH_TAG") == "clap" and "clap_chunk" or "vst_chunk"
  check("state-loaded", reaper.TrackFX_SetNamedConfigParm(track, fx, key, chunk), key .. " from " .. path)
end

-- ---- P1b lane RH: helpers for the product scenarios -------------------------------------------------------------

-- Decodes base64 text (the plugin state is a few KB, so a plain loop is fast enough).
local B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local B64_INDEX = {}
for i = 1, #B64 do B64_INDEX[B64:byte(i)] = i - 1 end
function b64_decode(text)
  local out, bits, count = {}, 0, 0
  for i = 1, #text do
    local v = B64_INDEX[text:byte(i)]
    if v then
      bits = (bits << 6) | v
      count = count + 6
      if count >= 8 then
        count = count - 8
        out[#out + 1] = string.char((bits >> count) & 255)
        bits = bits & ((1 << count) - 1)
      end
    end
  end
  return table.concat(out)
end

-- The plugin's saved state as decoded bytes; the JSON inside is plain text, so `contains` can look for a marker.
function state_bytes(track, fx) return b64_decode(state_chunk(track, fx)) end

function contains(text, marker) return text:find(marker, 1, true) ~= nil end

-- Writes the plugin's state as chunk-<tag>-<label>.b64 for the xtask to decode and compare.
function dump_chunk(label, track, fx)
  write_file("chunk-" .. os.getenv("ENTH_TAG") .. "-" .. label .. ".b64", state_chunk(track, fx))
end

local function exit_ok(a, b, c)
  if type(a) == "boolean" then return a end
  if type(a) == "number" then return a == 0 end
  return b == "exit" and c == 0
end

-- Runs xinput.py (the X11 helper next to this file; only ever under the Xvfb the xtask started) with `args`, a shell
-- string. Returns ok and its trimmed output.
function xinput(args)
  local out = OUT .. "/xinput.out"
  local cmd = "python3 '" .. os.getenv("ENTH_LUA_DIR") .. "/xinput.py' " .. args .. " > '" .. out .. "' 2>&1"
  local ok = exit_ok(os.execute(cmd))
  local f = io.open(out, "r")
  local text = f and f:read("*a") or ""
  if f then f:close() end
  return ok, (text:gsub("%s+$", ""))
end

-- Calls fn(true) as soon as `pred()` is true, or fn(false) after `timeout` seconds; polls every `step` seconds
-- (REAPER keeps running meanwhile). Scenarios wait for a state instead of sleeping a fixed time.
function wait_until(pred, timeout, step, fn)
  local t0 = reaper.time_precise()
  local function poll()
    local ok, done = pcall(pred)
    if ok and done then return safe(fn)(true) end
    if reaper.time_precise() - t0 >= timeout then return safe(fn)(false) end
    wait(step, poll)
  end
  poll()
end

-- The title REAPER gives the plugin's floating window starts with the name it lists the plugin by.
function editor_title() return os.getenv("ENTH_FX") end

-- Shows the plugin's floating window and calls fn(rendered, seconds) once the editor has drawn something: a window
-- that is viewable and has more than a handful of distinct colours (a blank window is a failure to render, not an
-- open editor). The wait is bounded, so a bigger or slower editor costs time and not a false failure. `title` picks the
-- window when several instances are open (REAPER titles them "<plugin> - Track N").
function show_editor(track, fx, fn, title)
  local t0 = reaper.time_precise()
  reaper.TrackFX_Show(track, fx, 3)
  wait_until(function()
    if reaper.TrackFX_GetFloatingWindow(track, fx) == nil then return false end
    local ok, text = xinput("stats " .. string.format("%q", title or editor_title()))
    local colours = tonumber(text:match("^(%d+)"))
    return ok and colours ~= nil and colours >= 20
  end, 15, 0.25, function(ok) fn(ok, reaper.time_precise() - t0) end)
end

function hide_editor(track, fx) reaper.TrackFX_Show(track, fx, 2) end

-- Blocks REAPER's script thread for `sec` seconds (for the few places that must act in sequence; the plugin's own
-- threads and the audio thread keep running).
function sleep(sec) os.execute("sleep " .. sec) end
