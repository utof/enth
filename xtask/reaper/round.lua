-- round (lane SKR, F2r): a Recreate round played through the plugin's own Recreate bar, with real X events
-- (xinput.py, under Xvfb only), the transport stopped. The project starts from Init (a fresh instance).
-- Checks:
--   round-prepared: Start, then the worker prepares the target: the saved state's round reaches `playing`;
--   target-sounds:  Target ▶ on, the track's peak meter read for 1.5 s is above silence (REAPER keeps calling
--                   process() while stopped: the loop plays);
--   mine-sounds:    Mine ▶ on (the phrase live through the start patch), the same;
--   submit-scored:  Round › Submit, then Skip on the blind rating: the saved round records a scored submit;
--   round-ended:    Round › End: the round is gone from the saved state and the round log has its `end` line.
-- Data: both peaks, the scored submit's text, the log line count. Audio is measured, never listened to.
--
-- The click points are offsets into the plugin's window (layout "lanes" at 1200 x 800). `reaper_points_hit_their_controls`
-- in crates/enth-ui/tests/recreate.rs asserts the same numbers against the editor, so a layout change fails there first.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local POINTS = {
  start = { 640, 28 },
  target = { 510, 28 },
  mine = { 576, 28 },
  round = { 636, 28 },
  submit = { 640, 61 },
  rate = { 624, 28 },
  skip = { 650, 218 },
  ["end"] = { 640, 145 },
}
local LOG = os.getenv("ENTH_ROUNDS_DIR") .. "/round-log.jsonl"

local function window()
  local ok, text = xinput("client " .. string.format("%q", editor_title()))
  if not ok then return nil end
  local x, y, w, h = text:match("^%d+ (%-?%d+) (%-?%d+) (%d+) (%d+)$")
  if not x then return nil end
  return tonumber(x), tonumber(y), tonumber(w), tonumber(h)
end

-- The round's phase in the saved `enth` field (its quotes may be escaped in the chunk).
local function phase_is(track, fx, phase)
  local s = state_bytes(track, fx)
  return contains(s, '"phase\\":\\"' .. phase) or contains(s, '"phase":"' .. phase)
end

local function has_round(track, fx)
  local s = state_bytes(track, fx)
  return contains(s, "round\\\":{") or contains(s, 'round":{')
end

-- The peak of the track's meter over `sec` seconds, then `fn(peak)`; never blocks REAPER's main thread.
local function peak_for(track, sec, fn)
  local peak, t0 = 0, reaper.time_precise()
  local poll
  poll = safe(function()
    for ch = 0, 1 do peak = math.max(peak, reaper.Track_GetPeakInfo(track, ch)) end
    if reaper.time_precise() - t0 < sec then return reaper.defer(poll) end
    fn(peak)
  end)
  reaper.defer(poll)
end

local function log_lines()
  local f = io.open(LOG, "r")
  if not f then return 0, "" end
  local text = f:read("*a")
  f:close()
  local n = 0
  for _ in text:gmatch("[^\n]+") do n = n + 1 end
  return n, text
end

safe(function()
  local track, fx = new_project_with_fx()
  if fx < 0 then return finish() end
  show_editor(track, fx, function(drew)
    check("editor-drew", drew, "the editor drew before the clicks")
    if not drew then return finish() end
    data("transport-playing", reaper.GetPlayState() & 1)
    local x, y, w, h = window()
    if not x then
      check("editor-window", false, "no plugin window found")
      return finish()
    end
    data("editor-window", string.format("%d,%d %dx%d", x, y, w, h))
    local function click(name)
      local p = POINTS[name]
      xinput(string.format("click %d %d", x + p[1], y + p[2]))
    end

    click("start")
    local t_start = reaper.time_precise()
    wait_until(function() return phase_is(track, fx, "playing") end, 30, 0.2, function(ready)
      check("round-prepared", ready, ready and string.format("the round plays %.1f s after Start", reaper.time_precise() - t_start)
        or "no round: the click points moved? see reaper_points_hit_their_controls in crates/enth-ui/tests/recreate.rs")
      if not ready then return finish() end
      data("prepare-s", string.format("%.2f", reaper.time_precise() - t_start))
      -- Let the start patch's plan and the meter settle, then Target ▶.
      wait(0.5, function()
        click("target")
        peak_for(track, 1.5, function(target_peak)
          data("target-peak", string.format("%.6f", target_peak))
          check("target-sounds", target_peak > 0.001, string.format("Target ▶ peaked at %.4f with the transport stopped", target_peak))
          click("mine")
          peak_for(track, 1.5, function(mine_peak)
            data("mine-peak", string.format("%.6f", mine_peak))
            check("mine-sounds", mine_peak > 0.001, string.format("Mine ▶ peaked at %.4f", mine_peak))
            click("mine") -- off again
            click("round")
            sleep(0.4)
            click("submit")
            sleep(0.6)
            click("rate")
            sleep(0.4)
            click("skip")
            wait_until(function()
              local s = state_bytes(track, fx)
              return contains(s, "at_ms")
            end, 30, 0.2, function(scored)
              check("submit-scored", scored, scored and "the saved round records a scored submit"
                or "no scored submit in the saved round")
              local s = state_bytes(track, fx)
              local at = s:find("submits", 1, true)
              data("submit", at and s:sub(at, at + 160):gsub("[\t\n]", " ") or "")
              sleep(0.3)
              click("round")
              sleep(0.4)
              click("end")
              wait_until(function() return not has_round(track, fx) end, 10, 0.2, function(ended)
                local n, text = log_lines()
                data("log-lines", n)
                check("round-ended", ended and n >= 1 and contains(text, '"enth.round-log/1"') and contains(text, '"event":"end"'),
                  string.format("round gone from the state: %s; %d line(s) in %s", tostring(ended), n, LOG))
                hide_editor(track, fx)
                wait(0.5, finish)
              end)
            end)
          end)
        end)
      end)
    end)
  end)
end)()
