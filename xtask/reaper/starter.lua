-- starter-project (P1b lane INT): the project `cargo xtask starter-project` writes opens in REAPER with every track's
-- enth loaded from the project's state. Opens ENTH_STARTER_PROJECT, checks the tracks, dumps each plugin's state
-- (starter-1 ...) and saves the project as starter-<tag>-saved.rpp, which the xtask runs `extract-presets` on.
dofile(os.getenv("ENTH_LUA_DIR") .. "/common.lua")

local tag = os.getenv("ENTH_TAG")
local project = os.getenv("ENTH_STARTER_PROJECT")
local saved = OUT .. "/starter-" .. tag .. "-saved.rpp"

safe(function()
  reaper.Main_openProject("noprompt:" .. project)
  wait(1.5, function()
    local count = reaper.CountTracks(0)
    local names = {}
    for i = 0, count - 1 do
      local track = reaper.GetTrack(0, i)
      local _, name = reaper.GetSetMediaTrackInfo_String(track, "P_NAME", "", false)
      names[#names + 1] = name
      local fx = reaper.TrackFX_GetCount(track)
      local _, fx_name = reaper.TrackFX_GetFXName(track, 0, "")
      check("track-" .. (i + 1) .. "-has-enth", fx == 1 and contains(fx_name, "enth"), name .. ": " .. fx .. " FX, " .. tostring(fx_name))
      if fx == 1 then dump_chunk("starter-" .. (i + 1), track, 0) end
    end
    data("starter-tracks", table.concat(names, ","))
    reaper.Main_SaveProjectEx(0, saved, 0)
    check("project-saved", reaper.file_exists(saved), saved)
    finish()
  end)
end)()
