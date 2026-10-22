# Installing enth (friend build)

enth is a synthesizer plugin. This is an early test build, and you may be the first person to run it on Windows or on a Mac, so expect rough edges and please tell me what breaks.

The zip holds the same plugin twice. **Install one of them**, the one your music program (DAW) prefers:

| File | Format |
| --- | --- |
| `enth-plugin.clap` | CLAP |
| `enth-plugin.vst3` | VST3 (on Windows and Mac this is a folder; keep it whole) |

| Your DAW | Use |
| --- | --- |
| Ableton Live, Cubase | VST3 |
| FL Studio | VST3 (CLAP also works in recent versions) |
| Studio One, Fender Studio Pro | VST3 or CLAP |
| Bitwig, REAPER | CLAP (VST3 also works) |
| Logic Pro, GarageBand | not supported yet (they only load Audio Units) |

## Windows

1. Right-click the zip, choose **Properties**, tick **Unblock** if you see it, press OK. Then right-click the zip and **Extract All**.
2. Press Win+R, type the folder below, press Enter (create it first if Windows says it is missing), and copy the file or folder in:
   - CLAP: `%LOCALAPPDATA%\Programs\Common\CLAP` gets `enth-plugin.clap`
   - VST3: `%LOCALAPPDATA%\Programs\Common\VST3` gets the whole `enth-plugin.vst3` folder
3. In your DAW, rescan plugins (or restart it). enth appears as an instrument.

If your DAW does not look in those folders, copy into `C:\Program Files\Common Files\CLAP` or `C:\Program Files\Common Files\VST3` instead (Windows asks for an administrator).

If Windows says "Part of this app has been blocked": that is Smart App Control, which refuses every program that has no paid signature, and this build has none. It cannot be allowed per file. Tell me; do not turn it off on my account.

## Mac (Apple silicon or Intel, macOS 11 or newer)

The build is not signed by Apple, so macOS refuses it until you clear one flag. "Open Anyway" in System Settings does **not** work for plugins. Open **Terminal**, type `cd ` (with a trailing space), drag the unzipped `enth-...` folder into the Terminal window, press Enter, then paste this:

```
xattr -dr com.apple.quarantine .
mkdir -p ~/Library/Audio/Plug-Ins/CLAP ~/Library/Audio/Plug-Ins/VST3
cp -R enth-plugin.clap ~/Library/Audio/Plug-Ins/CLAP/
cp -R enth-plugin.vst3 ~/Library/Audio/Plug-Ins/VST3/
```

Then rescan plugins in your DAW (or restart it). If the DAW still says it cannot verify the developer, run `xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins` once more and restart the DAW.

## Linux

Copy `enth-plugin.clap` to `~/.clap/` and the `enth-plugin.vst3` folder to `~/.vst3/`, then rescan in your DAW. It needs a 2024 or newer distribution (glibc 2.39: Ubuntu 24.04, Debian 13, Fedora 40 or newer); on an older one the DAW will not load it, so tell me.

## If your zip has `enth-bank-v1.bin`

Only builds with **Create** (make a patch from a sound file) have this 2.4 MB file. It helps Create find good starting patches; without it Create still works, just less well. Put it in an `enth/Models` folder in your Documents folder (create the folders): `Documents\enth\Models\` on Windows, `~/Documents/enth/Models/` on a Mac or Linux. On Windows and Linux, the folder you copied `enth-plugin.clap` into works too.

## First sound

Add enth as an instrument on a MIDI track and play some notes. **Start with your volume low**: the first sound can be loud.

## If something breaks

Send me:

- your DAW and its version, your OS version, and which file you installed (CLAP or VST3);
- what you did and what happened (a screenshot helps);
- the first two lines of `BUILD.txt` from the zip (they say which build you have).

Crashes: on a Mac the DAW's crash report is in `~/Library/Logs/DiagnosticReports`; on Windows tell me when it happened and what you were doing.

After installing, play the Recreate rounds as [gr1-friends.md](gr1-friends.md) describes (the zip holds a copy). For the lessons, [gr2-friends.md](gr2-friends.md) (also in the zip) says how to add the zip's `Packs` folder and play the tour.
