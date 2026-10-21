-- lesson (lane SKL, F2l; lane LUI's view): the bundled example lesson (`example/cutoff`: Listen, Do, Choose) played
-- through the plugin's Learn view with real X events (xinput.py, under Xvfb only), on a bundle built with the `learn` feature, the
-- transport stopped. Two launches: `play` starts the lesson and passes its Do step, then saves the project mid-lesson;
-- `reload` opens the saved project in a new REAPER process and goes on from the saved step.
-- Checks (play):
--   lesson-started:  Learn, then Start: the saved `enth` field holds the lesson (`learn` object, step 0) and the stash;
--   example-sounds:  ▶ Open (the Listen step's first example, rendered on the game worker and played from the target
--                    slot): the track's peak meter over 1.5 s is above silence;
--   step-next:       Next: the saved lesson is at step 1 (the Do step);
--   step-passed:     Show me (the witness's edit, through the session's edit path), then the sound check rendered on
--                    the worker passes: the progress file (ENTH_LEARN_DIR) records step `try` done;
--   project-saved:   the project saved mid-lesson.
-- Checks (reload):
--   reload-lesson:   the saved lesson is in the reloaded state at step 1 with its stash;
--   lesson-resumed:  the editor opens, the lesson reopens on the worker (Open, then Player::resume and the progress file
--                    read back), and Next moves it to step 2: only a resumed player answers Next;
--   no-crash:        the crash folder is still empty.
-- Data: the example's peak, the times. Audio is measured, never listened to.
--
-- The click points are offsets into the plugin's window (1200 x 800, the plugin's Init patch; lane LUI's Learn view:
-- the browser's first row on the example tour, then the pane's controls, anchored to its bottom):
-- `reaper_points_hit_their_controls` in crates/enth-ui/tests/learn_panel.rs asserts the same numbers.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local POINTS = {
  learn = { 551, 28 },
  start = { 901, 160 },
  listen_open = { 910, 580 },
  stop = { 908, 608 },
  next = { 1155, 636 },
  show = { 957, 608 },
}
local tag = os.getenv("ENTH_TAG")
local phase = os.getenv("ENTH_PHASE")
local project = OUT .. "/lesson-" .. tag .. ".rpp"
local PROGRESS = os.getenv("ENTH_LEARN_DIR") .. "/progress.json"
local CRASHES = os.getenv("ENTH_CRASH_DIR")

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

-- The saved lesson's step, read from the `enth` field (its quotes may be escaped in the chunk), or nil.
local function saved_step(track, fx)
  local s = state_bytes(track, fx)
  local at = s:find('"lesson', s:find('"learn', 1, true) or #s + 1, true)
  if not at then return nil end
  local step = s:match('"step\\?":(%d+)', at)
  return step and tonumber(step)
end

local function has_stash(track, fx)
  local s = state_bytes(track, fx)
  return contains(s, '"stash\\":') or contains(s, '"stash":')
end

local function progress_text()
  local f = io.open(PROGRESS, "r")
  if not f then return "" end
  local text = f:read("*a")
  f:close()
  return text
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

-- Shows the editor, finds its window and calls fn(click) with a function that clicks a named point.
local function with_editor(track, fx, fn)
  show_editor(track, fx, function(drew)
    check("editor-drew", drew, "the editor drew before the clicks")
    if not drew then return finish() end
    local x, y, w, h = window()
    if not x then
      check("editor-window", false, "no plugin window found")
      return finish()
    end
    data("editor-window", string.format("%d,%d %dx%d", x, y, w, h))
    fn(function(name)
      local p = POINTS[name]
      xinput(string.format("click %d %d", x + p[1], y + p[2]))
    end)
  end)
end

if phase == "play" then
  safe(function()
    local track, fx = new_project_with_fx()
    if fx < 0 then return finish() end
    with_editor(track, fx, function(click)
      click("learn")
      sleep(0.4)
      click("start")
      local t0 = reaper.time_precise()
      wait_until(function() return saved_step(track, fx) == 0 and has_stash(track, fx) end, 15, 0.2, function(started)
        check("lesson-started", started, started and string.format("the saved state holds the lesson at step 0 after %.1f s", reaper.time_precise() - t0)
          or "no lesson in the saved state: the click points moved? see reaper_points_hit_their_controls in crates/enth-ui/tests/learn_panel.rs")
        if not started then return finish() end
        -- The examples render on the worker (debug bundle: well under a second); then ▶ Open.
        wait(1.5, function()
          click("listen_open")
          peak_for(track, 1.5, function(peak)
            data("example-peak", string.format("%.6f", peak))
            check("example-sounds", peak > 0.001, string.format("▶ Open peaked at %.4f with the transport stopped", peak))
            click("stop")
            sleep(0.3)
            click("next")
            wait_until(function() return saved_step(track, fx) == 1 end, 10, 0.2, function(moved)
              check("step-next", moved, moved and "the saved lesson is at step 1" or "Next did not move the lesson")
              if not moved then return finish() end
              sleep(0.5)
              click("show")
              local t_show = reaper.time_precise()
              -- The settle moment (300 ms), then the sound check on the worker; the pass is merged into the file at once.
              wait_until(function()
                local text = progress_text()
                return contains(text, '"try"') and contains(text, '"done": true')
              end, 20, 0.2, function(passed)
                check("step-passed", passed, passed and string.format("the progress file records step try %.1f s after Show me", reaper.time_precise() - t_show)
                  or ("no pass in " .. PROGRESS .. ": " .. progress_text():gsub("%s+", " ")))
                data("progress", (progress_text():gsub("%s+", " ")))
                hide_editor(track, fx)
                wait(0.5, function()
                  dump_chunk("lesson-mid", track, fx)
                  reaper.Main_SaveProjectEx(0, project, 0)
                  check("project-saved", reaper.file_exists(project), project)
                  finish()
                end)
              end)
            end)
          end)
        end)
      end)
    end)
  end)()
else
  safe(function()
    reaper.Main_openProject("noprompt:" .. project)
    local track = reaper.GetTrack(0, 0)
    wait(1.5, function()
      local fx = 0
      check("reload-fx", track ~= nil and reaper.TrackFX_GetCount(track) == 1, "one plugin on track 1")
      if not track then return finish() end
      local step = saved_step(track, fx)
      check("reload-lesson", step == 1 and has_stash(track, fx), string.format("saved step %s, stash %s", tostring(step), tostring(has_stash(track, fx))))
      with_editor(track, fx, function(click)
        -- The pane opens by itself on the resumed lesson; give the Open job a moment, then Next.
        wait(1.0, function()
          click("next")
          wait_until(function() return saved_step(track, fx) == 2 end, 10, 0.2, function(resumed)
            check("lesson-resumed", resumed, resumed and "the reopened lesson moved to step 2"
              or string.format("the lesson did not move on after the reload (saved step %s)", tostring(saved_step(track, fx))))
            local crashes = crash_files()
            check("no-crash", crashes == 0, string.format("%d crash file(s) in %s", crashes, tostring(CRASHES)))
            dump_chunk("lesson-resumed", track, fx)
            hide_editor(track, fx)
            wait(0.5, finish)
          end)
        end)
      end)
    end)
  end)()
end
