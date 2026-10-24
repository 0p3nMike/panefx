# Changelog

## Unreleased -- deskfx animations as wallpaper loops (branch `deskfx-loops`)

### Added
- **deskfx animations on the desktop.** A wallpaper effect `deskfx:NAME`
  (16 animations: flames, rain, torus, torus-rt, sphere, arch, groovy,
  blackwaves, amtui-flames, fire, waves, plasma, plasma-square, tunnel,
  starfield, cube) plays a loop recorded by deskfx's own recorder through the
  baked-loop player. Loops live in `%LOCALAPPDATA%\panefx\loops\deskfx\`
  as `WIDTHxHEIGHT\THEME\NAME@PCT.mp4`: one per screen size, theme
  (`theme`) and detail level (`wallpaper_detail` 1..10 = glyph size
  200 150 125 100 85 70 60 50 40 33 %). They are only played, never drawn:
  `visualizer` does not apply to them.
  - A missing loop shows the plain theme background, is recorded in the
    background when WSL has `deskfx-bake` (tools/deskfx-bake), and starts
    playing within 5 s of landing.
  - panefx writes `loops\deskfx\screens` (each monitor's size and DPI
    scale), which the recorder reads.
  - A theme change re-picks every deskfx monitor's loop.
- **panefx-ctl: animations page.** `a` on the Themes tab: pick the animation
  (Enter, every monitor), `-`/`+` detail, `v` visualizer, `p` pause switch,
  `b` bake the animation at every detail level; `a`/Esc back. Shows whether
  the loop at this detail is baked for every screen size, and its glyph count.
- **tools/deskfx-bake**: records the loops in WSL with c4nvas's `bake`
  (needs c4nvas, a Linux deskfx build, python3-gi/cairo, ffmpeg).

## Unreleased -- baked wallpaper loops (branch `baked-loops`)

### Added
- **Baked loops.** With the new **visualizer** switch off (the default) each
  screen's wallpaper is recorded once into a seamless 30 fps H.264 loop and
  played back by the GPU's video decoder through Windows' own media engine,
  instead of being drawn with GDI every frame. A look that has no loop yet is
  drawn live while it is recorded in the background (after it has been
  unchanged for 8 s, one recording at a time, at background priority).
  - Periodic effects (plasma, tunnel, donut/sphere/cube/galaxy) are recorded
    for exactly one period; the others warm up 5 s, record 20 s and crossfade
    the last 1.5 s into the opening.
  - One loop per look: effect stack, theme colours and params, glyph size
    (`wallpaper_detail`), monitor resolution and fps. Files live in
    `%LOCALAPPDATA%\panefx\loops`, named `EFFECT@CELL-RESOLUTION-HASH.mp4`
    with the full recipe beside each in a `.txt`. Deleting the folder only
    costs re-recording.
  - A playing loop is never paused when windows cover the desktop.
- **visualizer** switch (tray, GUI, TUI, `visualizer = true|false`): on = draw
  live every frame, as before. Nothing flips it automatically.
- The snapshot (`panefx-ctl`/`get`) reports each monitor's `playback`.

### Measured (2026-10-23)
- A four-monitor desktop (RTX 5090), four screens of plasma all visible: live
  44-49% of one core, baked 11-12%. DWM about the same either way (32% live,
  28% baked).
- A laptop on Intel integrated graphics, one 2400x1600 screen of flames: baked
  21-25% at 30 fps, 12.6% at `wallpaper_fps = 15`; live 14-19%, but live only
  delivers ~14 fps
  there (its 33 ms frame budget misses every other 31 ms daemon tick at
  `fps = 32`; not changed here), so per frame baked is
  ~2.5x cheaper while in total it costs more, and it does not pause.

### Fixed
- Changing `wallpaper_fps` live now rebuilds the wallpaper, so the new rate
  reaches the simulations (and the loops) instead of waiting for a restart.

## 0.2.1 -- switches and wallpaper layers (2026-10-08)

### Added
- **Three switches**, in the GUI's settings tab, the TUI, and ticked in the tray
  menu. Each saves itself the moment it is flipped:
  - **backdrops** -- the effects behind your terminal windows (as before).
  - **transparency** -- see-through windows everywhere, on or off. Off makes
    Alacritty and Neovide solid while the backdrops keep running; your chosen
    opacity is kept for when it goes back on.
  - **pause when covered** -- stop animating a screen while windows cover it
    (saves CPU), or keep it animating always.
- Default window opacity is 70% (was 60%). Existing configs keep their value.

### Fixed
- **A wallpaper layer could get stuck on a screen.** The UIs address a layer by
  its position in the stack, but the stored layer numbers kept gaps and kept
  layers above a screen that was switched off, so the two drifted apart: a
  plasma layer stored as "layer 2" over an `off` base showed at position 0,
  "remove" asked for the base instead, and the plasma could never be removed --
  every base chosen afterwards drew under it. Layers are now always numbered
  1, 2, 3... with no gaps, switching a screen off clears its layers, adding a
  layer to an off screen makes it the base, and a config written before this is
  repaired as it loads.
- **Changing a screen's base effect drew the old one.** The screen was rebuilt
  from the config before the new choice was saved into it, so switching
  plasma -> flames kept drawing plasma. The config is now updated first (and
  rolled back if the screen refuses), and the base is always the effect chosen.
- **Without GlazeWM, panefx no longer freezes for 4 seconds at a time.** It
  tried to reconnect to a GlazeWM that was not running every 10 s, on the thread
  that draws everything, and a refused localhost connection takes 4 s on
  Windows. It now only connects when a GlazeWM process exists -- which also
  takes 4 s off every startup on a machine without GlazeWM.
- GlazeWM quitting at the wrong moment could stop the whole daemon; it now
  falls back to following windows through Windows itself, as it already did
  otherwise.
- `revert` made the windows see-through even with transparency or the
  backdrops switched off.
- Clicking a switch in the GUI hid any unsaved changes (the save/revert buttons
  vanished), and the TUI marked self-saving switches as "*modified".
- The tray's Reload could leave no daemon running: the new one was refused by
  the old one's lock while it was still exiting.
- Saving a switch or a theme could write `PANEFX_*` environment overrides into
  config.toml; config.toml is now written atomically.
- In the TUI, cycling a layered screen's effect no longer passes through `off`
  (which would clear its layers).
- Explorer's desktop window can never count as covering a screen.
- The settings tab's hints for "freeze at %" and "bg opacity %" carried stray
  line breaks and indentation.
- The log says "standing in for GlazeWM" when GlazeWM goes away, not "by
  configuration".

### Added
- `drawing` in each monitor's snapshot: the simulations the screen is really
  running, beside `layers`, the config's view of them.

## 0.2.0 -- themes (2026-09-30)

### Added
- **The theme driver.** 27 colour themes plus *house* (the look you dialled in by
  hand, recorded the moment a theme first replaces it and restored exactly).
  One pick recolours panefx's effects -- every colour of every effect, behind the
  terminals and on every monitor, live -- and its GUI, then Alacritty, the Neovim
  editors, Windows light/dark, VS Code, Windows Terminal, Xournal++ and the
  sl0p.notepad / sl0p.ink forks. Each app reports what happened to it, can be
  switched off, and an app that was open (Xournal++) is retried until it closes.
- Themes tab in the GUI and the TUI (`t`, `n`/`N` for next/previous); a Theme
  submenu plus next/previous in the tray; `panefx theme <id|next|prev|house|list>`,
  which also works with the daemon stopped.
- `{"cmd":"theme","name":...}` on the control channel.
- Your own themes in `~\.config\panefx\themes\`.
- `extras/nvim/zz-colormesh.lua`, THEMES.md.

### Fixed
- The GUI's settings tab scrolls; on a small window its lower half was unreachable.
- The GUI's logs tab shows the log (a `logs` reply was being dropped).
- `panefx --tui` no longer trips panefx-ctl's unknown-option check.
- `build.ps1 -Install` restarts the daemon through the `panefx` logon task rather
  than as a child of the build shell.
- Only one GUI opens, however fast the tray icon is clicked.
- GlazeWM is optional everywhere: the daemon no longer exits when GlazeWM quits
  (it follows windows through Win32 and picks GlazeWM back up), and the GUI no
  longer claims the terminal backdrops need it.

### Removed
- The effects built from other people's art: wizardtorch, tgevil, wzfire,
  raalien, fishloop, warlockspin.

## 0.1.0

The follower, the desktop wallpaper with per-monitor effects and layers, the TUI,
the Win98 GUI, the tray, the installer and the report tool.
