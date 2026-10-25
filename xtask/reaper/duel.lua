-- duel (lane SKD, F2d): a duel between two instances of the plugin on two tracks, played through their duel cards with
-- real X events (xinput.py, under Xvfb only), the transport stopped. The codes travel through the X clipboard: Copy on
-- one card, then the platform's Paste (Ctrl+V) into the other card's paste box. Needs a bundle built with the `duel`
-- feature (`cargo xtask reaper-test --features duel`).
-- Instance A (track 1) starts from a saved round whose patch is its target (ENTH_STATE_CHUNK_FILE); B (track 2) from Init.
-- Checks:
--   a-ready:         A's restored round: Target ▶ sounds on track 1 once the worker has prepared the target again;
--   a-passed:        Submit, Skip on the blind rating: the saved round has passed;
--   a-shared:        Duel, Share result: the duel log has A's codes (a scorecard and an answer code, made on A's worker
--                    with the salt drawn from OS randomness);
--   b-card:          A's Copy message, then B's Duel, paste box, Ctrl+V, Read: B holds A's scorecard as reported;
--   b-sealed:        A's Copy answer, then B's **Paste** button (lane DUI: the card asks for the clipboard with egui's
--                    RequestPaste, which fork patch 13 answers; what arrives is read at once): A's answer is held,
--                    sealed (B has not played);
--   b-played:        B's Play (a round on the duel's challenge), Target ▶ until it sounds, Reveal (B gives up): the
--                    answer opens;
--   b-checked:       B's Verify: checked on B's worker, "as reported", the commitment matched, their patch at hand;
--   no-crash:        the crash folder is still empty.
-- Data: the codes' sizes, the times. Audio is measured, never listened to.
--
-- The click points are offsets into a plugin window (layout "lanes" at 1200 x 800): the Recreate bar's from
-- round.lua (`reaper_points_hit_their_controls` in crates/enth-ui/tests/recreate.rs), the duel card's and Reveal's from
-- `reaper_points_hit_their_duel_controls` in crates/enth-ui/tests/duel.rs, which asserts the same numbers.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local POINTS = {
  target = { 174, 28 },
  submit = { 402, 28 },
  skip = { 380, 708 },
  reveal = { 505, 28 },
  opener = { 32, 746 },
  share = { 902, 104 },
  copy_message = { 1002, 104 },
  copy_answer = { 1105, 104 },
  paste = { 1020, 156 },
  paste_button = { 1156, 132 },
  read = { 882, 208 },
  verify = { 937, 208 },
  play = { 880, 236 },
}
local LOG = os.getenv("ENTH_DUEL_LOG")
local CRASHES = os.getenv("ENTH_CRASH_DIR")
local TITLE_A = editor_title() .. " - Track 1"
local TITLE_B = editor_title() .. " - Track 2"

local function crash_files()
  local n = 0
  if not CRASHES then return 0 end
  while reaper.EnumerateFiles(CRASHES, n) do n = n + 1 end
  return n
end

local function log_text()
  local f = io.open(LOG, "r")
  if not f then return "" end
  local text = f:read("*a")
  f:close()
  return text
end

-- Whether one line of the duel log holds every marker.
local function logged(...)
  local markers = { ... }
  for line in log_text():gmatch("[^\n]+") do
    local all = true
    for _, m in ipairs(markers) do
      if not contains(line, m) then all = false break end
    end
    if all then return true, line end
  end
  return false, nil
end

-- The plugin window titled `title`: x, y of its client area.
local function window(title)
  local ok, text = xinput("client " .. string.format("%q", title))
  if not ok then return nil end
  local x, y = text:match("^%d+ (%-?%d+) (%-?%d+) %d+ %d+$")
  if not x then return nil end
  return { x = tonumber(x), y = tonumber(y), title = title }
end

-- Raise the window (two floating windows may overlap: there is no window manager), then click a point in it.
local function click(win, name)
  xinput("raise " .. string.format("%q", win.title))
  local p = POINTS[name]
  xinput(string.format("click %d %d", win.x + p[1], win.y + p[2]))
  sleep(0.3)
end

-- The platform's Paste into the paste box: click it (the box takes the keyboard while it is focused), then Ctrl+V.
local function paste_and_read(win)
  click(win, "paste")
  sleep(0.3)
  xinput("chord Control_L v")
  sleep(0.4)
  click(win, "read")
end

local function phase_is(track, fx, phase)
  local s = state_bytes(track, fx)
  return contains(s, '"phase\\":\\"' .. phase) or contains(s, '"phase":"' .. phase)
end

-- The track's peak meter over `sec` seconds, then `fn(peak)`; never blocks REAPER's main thread.
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

-- Target ▶ is disabled until the round's target is prepared: click it until the track sounds (at most `tries`
-- times), then click it off again, then fn(sounded).
local function until_target_sounds(win, track, tries, fn)
  click(win, "target")
  peak_for(track, 1.0, function(peak)
    if peak > 0.001 then
      click(win, "target")
      return fn(true, peak)
    end
    if tries <= 1 then return fn(false, peak) end
    until_target_sounds(win, track, tries - 1, fn)
  end)
end

safe(function()
  local ta, fa = new_project_with_fx()
  if fa < 0 then return finish() end
  reaper.InsertTrackAtIndex(1, true)
  local tb = reaper.GetTrack(0, 1)
  local fb = reaper.TrackFX_AddByName(tb, os.getenv("ENTH_FX"), false, -1)
  check("load-second", fb >= 0, "second instance on track 2")
  if fb < 0 then return finish() end
  inject_state(ta, fa)
  check("a-round-loaded", phase_is(ta, fa, "playing"), "A's saved round is in its state")
  data("transport-playing", reaper.GetPlayState() & 1)
  show_editor(ta, fa, function(drew_a)
    check("a-editor-drew", drew_a, "A's editor drew")
    local a = window(TITLE_A)
    if not (drew_a and a) then return finish() end
    data("a-window", string.format("%d,%d", a.x, a.y))
    until_target_sounds(a, ta, 30, function(ready, peak)
      check("a-ready", ready, string.format("Target ▶ on track 1 peaked at %.4f once the target was prepared again", peak))
      if not ready then return finish() end
      click(a, "submit")
      sleep(0.6)
      click(a, "skip")
      wait_until(function() return phase_is(ta, fa, "passed") end, 30, 0.2, function(passed)
        check("a-passed", passed, passed and "A's saved round has passed" or "no pass in A's saved round")
        if not passed then return finish() end
        click(a, "opener")
        click(a, "share")
        local t_share = reaper.time_precise()
        wait_until(function() return logged('"event":"share"') end, 30, 0.2, function(shared)
          local _, line = logged('"event":"share"')
          check("a-shared", shared, shared and string.format("A's codes made in %.1f s", reaper.time_precise() - t_share)
            or "no codes in the duel log: the click points moved? see reaper_points_hit_their_duel_controls")
          if not shared then return finish() end
          data("share-s", string.format("%.2f", reaper.time_precise() - t_share))
          data("scorecard-symbols", line:match('"scorecard_symbols":(%d+)') or "")
          data("answer-symbols", line:match('"answer_symbols":(%d+)') or "")
          click(a, "copy_message")
          -- B's editor opens beside A's (A's stays open: its window serves the clipboard).
          show_editor(tb, fb, function(drew_b)
            check("b-editor-drew", drew_b, "B's editor drew")
            local b = window(TITLE_B)
            if not (drew_b and b) then return finish() end
            data("b-window", string.format("%d,%d", b.x, b.y))
            click(b, "opener")
            paste_and_read(b)
            wait_until(function() return logged('"card":true', '"duel":true') end, 10, 0.2, function(card)
              check("b-card", card, card and "B holds A's scorecard, as reported by A's app"
                or "no scorecard in B's duel: the paste did not arrive?")
              if not card then return finish() end
              click(a, "copy_answer")
              -- The card's Paste button this time (lane DUI): one click reads the clipboard and the paste box's text.
              click(b, "paste_button")
              wait_until(function() return logged('"answer":"sealed"') end, 10, 0.2, function(sealed)
                check("b-sealed", sealed, sealed and "A's answer, pasted with the Paste button, is held sealed: B has not played"
                  or "no sealed answer in B's duel (the Paste button: fork patch 13's RequestPaste)")
                if not sealed then return finish() end
                click(b, "play")
                until_target_sounds(b, tb, 30, function(b_ready, b_peak)
                  check("b-round", b_ready, string.format("B's round on the duel's challenge: Target ▶ peaked at %.4f", b_peak))
                  if not b_ready then return finish() end
                  click(b, "reveal")
                  wait_until(function() return logged('"answer":"open"') end, 10, 0.2, function(open)
                    check("b-played", open, open and "B gave up: A's answer opens" or "the answer stayed sealed")
                    if not open then return finish() end
                    click(b, "verify")
                    local t_verify = reaper.time_precise()
                    wait_until(function()
                      return logged('"verify":"done"', '"as_reported":true', '"commitment_ok":true', '"has_patch":true')
                    end, 30, 0.2, function(checked)
                      local _, l = logged('"verify":"done"')
                      check("b-checked", checked, checked
                        and string.format("checked on B's computer in %.1f s: as reported, the commitment matched",
                          reaper.time_precise() - t_verify)
                        or ("no check in the duel log: " .. tostring(l)))
                      data("verify-s", string.format("%.2f", reaper.time_precise() - t_verify))
                      local crashes = crash_files()
                      check("no-crash", crashes == 0, string.format("%d crash file(s) in %s", crashes, tostring(CRASHES)))
                      hide_editor(tb, fb)
                      hide_editor(ta, fa)
                      wait(0.5, finish)
                    end)
                  end)
                end)
              end)
            end)
          end, TITLE_B)
        end)
      end)
    end)
  end, TITLE_A)
end)()
