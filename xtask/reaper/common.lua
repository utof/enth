-- Shared helpers for the headless REAPER tests run by `cargo xtask reaper-test`.
-- Settings come from environment variables set by the xtask:
--   ENTH_OUT (results dir), ENTH_EMPTY_PROJECT, ENTH_LOG (log file name inside ENTH_OUT),
--   ENTH_FX (REAPER plugin name, e.g. "CLAPi: enth spike-shell"), ENTH_TAG (e.g. "clap"), ENTH_PHASE.
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
