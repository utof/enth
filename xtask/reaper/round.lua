-- round (lane SKR, F2r): a Recreate round played through the plugin's own Recreate bar, with real X events
-- (xinput.py, under Xvfb only), the transport stopped. The project starts from Init (a fresh instance).
-- Checks:
--   round-prepared: Recreate, then Start (lane RUI's challenge bar), then the worker prepares the target: the saved
--                   state's round reaches `playing`;
--   target-sounds:  Target ▶ on, the track's peak meter read for 1.5 s is above silence (REAPER keeps calling
--                   process() while stopped: the loop plays);
--   mine-sounds:    Mine ▶ on (the phrase live through the start patch), the same;
--   submit-scored:  Submit, then Skip on the blind rating (in the results drawer): the saved round records a scored
--                   submit;
--   hint-asked:     Hint (lane HINT, FU1): the saved round records hint level 1, and the coach's job runs on the game
--                   worker's parallel evaluator (scoped threads inside the host) while the round goes on;
--   hint-no-crash:  a few seconds later the crash folder is still empty (a panicking render would write a file);
--   round-ended:    End › End and restore my patch: the round is gone from the saved state and the round log has its
--                   `end` line.
-- Data: both peaks, the scored submit's text, the log line count. Audio is measured, never listened to.
--
-- The click points are offsets into the plugin's window (layout "lanes" at 1200 x 800). `reaper_points_hit_their_controls`
-- in crates/enth-ui/tests/recreate.rs asserts the same numbers against the editor, so a layout change fails there first.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local POINTS = {
  recreate = { 628, 28 },
  start = { 353, 28 },
  target = { 174, 28 },
  mine = { 240, 28 },
  submit = { 402, 28 },
  skip = { 380, 708 },
  hint = { 454, 28 },
  ["end"] = { 554, 28 },
  -- After a hint, "1/4" stands beside Hint and End moves right.
  end_hinted = { 574, 28 },
  restore = { 620, 61 },
}
local LOG = os.getenv("ENTH_ROUNDS_DIR") .. "/round-log.jsonl"
local CRASHES = os.getenv("ENTH_CRASH_DIR")

-- How many files the crash folder holds (the plugin's panic hook writes one per crash).
local function crash_files()
  local n = 0
  if not CRASHES then return 0 end
  while reaper.EnumerateFiles(CRASHES, n) do n = n + 1 end
  return n
end

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

    click("recreate")
    sleep(0.4)
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
            click("submit")
            sleep(0.6)
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
              click("hint")
              local t_hint = reaper.time_precise()
              wait_until(function()
                local st = state_bytes(track, fx)
                return contains(st, '"hints\\":1') or contains(st, '"hints":1')
              end, 10, 0.2, function(asked)
                check("hint-asked", asked, asked and string.format("the saved round records hint level 1 after %.1f s", reaper.time_precise() - t_hint)
                  or "no hint in the saved round: the click point moved? see reaper_points_hit_their_controls")
                -- Give the coach's job time to run on the worker's threads (debug bundle), then look for a crash file.
                wait(6, function()
                  local crashes = crash_files()
                  data("crash-files", crashes)
                  check("hint-no-crash", crashes == 0, string.format("%d crash file(s) in %s after the hint", crashes, tostring(CRASHES)))
                  click("end_hinted")
                  sleep(0.4)
                  click("restore")
                  -- The state loses the round under the session's lock, and the plugin appends the log line only after
                  -- it unlocks (20 to 140 ms later in ten VST3 runs at load 20), so a poll that reads the state first can
                  -- see the round gone before the line exists ("gone: true; 0 line(s)" in the coordinator's full run).
                  -- Wait for both.
                  wait_until(function() return not has_round(track, fx) and log_lines() >= 1 end, 10, 0.2, function(done)
                    local ended = not has_round(track, fx)
                    local n, text = log_lines()
                    data("log-lines", n)
                    check("round-ended", ended and n >= 1 and contains(text, '"enth.round-log/1"') and contains(text, '"event":"end"'),
                      string.format("round gone from the state: %s; %d line(s) in %s", tostring(ended), n, LOG))
                    check("hint-in-log", contains(text, '"hints":1'), "the end line's assists count the hint")
                    -- What a click met (a stray window over the editor, the End menu not open) is in the picture.
                    if not done then shot("round-end-failed", true) end
                    hide_editor(track, fx)
                    wait(0.5, finish)
                  end)
                end)
              end)
            end)
          end)
        end)
      end)
    end)
  end)
end)()
