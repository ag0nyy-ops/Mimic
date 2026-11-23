# Mimic

Mimic a game on your discord rich presence, so you can be the best larp in the world.

Pick any game from discord's own list, write what your profile should say (details, state, timer, party size, buttons) and go live. A live preview card shows what it will look like while you type.

> Screenshots are coming. Placeholders for now:
>
> - `docs/screenshots/main-window.png`: editor with the live preview card
> - `docs/screenshots/game-picker.png`: the searchable game picker
> - `docs/screenshots/tray.png`: tray menu with quick switch

**Status:** the Windows app is done and ready for testing. macOS and Linux versions are still to come, each in its own folder; the spec in [`shared/`](shared/) is written so every app can use it.

## What it does

- Searchable picker over about 25,000 games, taken from discord's public "detectable applications" list (never typed in by hand).
- Editable details, state, large and small image keys with hover text, elapsed timer (none, start now, or a custom offset), party size and up to two buttons.
- Saved presets, quick switch from the tray, auto-restore of the last preset on launch.
- Real error states: discord not running, bad application id, rate limited (with a countdown), invalid button urls.
- Tray icon, minimize to tray, close to tray, launch at login.
- Updates are debounced to stay inside discord's rate limit of about one presence update per 15 seconds.

## Install

Windows only for now.

### Trying a build from CI (what you want while testing)

1. Open the repo's **Actions** tab, then the latest green **build-windows** run.
2. Scroll to **Artifacts** and download `mimic-windows-installers`.
3. Unzip it. You get an `.exe` (NSIS) and an `.msi`. Either one works, use the `.exe` if you're unsure.
4. Run it. The installers aren't code signed, so Windows SmartScreen will say "Windows protected your PC": click **More info**, then **Run anyway**.

The NSIS installer installs for the current user only and doesn't need admin rights.

### From a release

Releases are only made by pushing a `v*` tag, and they're created as drafts first. When one is published you'll find the same two installers on the releases page.

## Using it

1. Start the **discord desktop app** (the browser version has no local ipc, so it can't work).
2. Open mimic, click **New preset**, search for a game and pick it.
3. Fill in what you want. The card on the right updates as you type.
4. Hit **Go live**. Check your profile from another account to see buttons, since discord never shows them on your own.
5. **Stop & clear presence** ends it. Closing the window keeps mimic running in the tray.

## How it works

Discord's desktop client opens a local "pipe" on your machine, and any program can talk to it to say "I'm a game, here's my status". That's all rich presence is. mimic speaks that protocol directly (there's no discord library in here). The full spec is in [`shared/README.md`](shared/README.md), this is the short version.

**Finding discord.** On Windows the pipe is called `\\.\pipe\discord-ipc-0`. If discord already holds number 0 (a second discord running, say), it uses 1, then 2, up to 9. mimic tries 0 through 9 and takes the first one that answers.

**Frames.** Everything sent over the pipe is wrapped in a frame: 4 bytes saying what kind of message it is, 4 bytes saying how long it is (both little endian), then that many bytes of json. The kinds are handshake (0), normal message (1), close (2), ping (3) and pong (4).

**Handshake.** The first frame mimic sends is a handshake: `{"v":1,"client_id":"<application id>"}`. The application id is what makes discord say "Playing Minecraft" instead of "Playing mimic": every game on discord's list has one. Discord answers with a `READY` message, and only then is it safe to send anything. If the id is no good you get a close frame or an error instead, which mimic shows as "bad application id".

**Setting the presence.** Once ready, mimic sends a `SET_ACTIVITY` command with the details, state, timer, party and buttons, plus its own process id and a random id so replies can be matched up. Clearing a presence is the same command with the activity set to `null`.

**One game, one connection.** A connection belongs to the application id it said hello with. To switch games, mimic closes the connection and opens a new one with the new id. Discord removes a presence as soon as its connection closes, which is also how **Stop** works.

**Keeping it alive.** mimic answers pings, and if the pipe goes away (discord quit or restarted) it waits 1 second, then 2, 4, 8 and so on up to 30, trying all ten pipes each time, and puts your presence back as soon as discord returns.

## Good to know

- **Image keys.** Discord only shows an image if the application you're pretending to be owns an asset with that exact key. For games from discord's list that application belongs to the game's developer, so keys you make up usually show nothing. Whether a full image url works as a key over ipc is **untested**, don't count on it.
- **Buttons** are only visible to other people, never on your own profile.
- **Rate limit.** Edits are collapsed into one update, sent after you stop typing for about a second, and never more often than every 15 seconds. The first update after connecting goes out immediately. While an update is waiting, the app shows a countdown.
- **Stop** closes the connection rather than sending a "clear" command, because a clear would be stuck behind that rate limit while closing the connection removes the presence right away.
- Your elapsed timer keeps running while you edit text. It only restarts when you change the timer setting itself.
- The game list that ships in the app is a snapshot. **Settings > Refresh game list** downloads a fresh one from discord and keeps it on disk.
- Only one presence is shown at a time.

## Dev setup (Windows)

You need:

- [Rust](https://rustup.rs) (stable) with the MSVC toolchain, which needs the "Desktop development with C++" part of the Visual Studio Build Tools
- [Node.js](https://nodejs.org) 22 or newer
- WebView2, which comes with Windows 11 and current Windows 10 ([download](https://developer.microsoft.com/microsoft-edge/webview2/) if missing)

Then, from the `windows/` folder:

```sh
npm install
npm run tauri dev
```

To build the installers yourself:

```sh
npm run tauri build
```

They end up in `windows/target/release/bundle/nsis/` and `windows/target/release/bundle/msi/`.

For work on the ui alone you can run `npm run dev` and open the printed address in a normal browser. It uses a fake in-memory backend (dev builds only) so you can click around without tauri or discord. Add `?live=1`, `?state=offline`, `?state=badapp`, `?state=rate` or `?empty=1` to the address to see the different states.

### Tests

From `windows/`:

```sh
cargo test              # the core crate: runs on any os, no windows or webview libs needed
cargo test --workspace  # also compiles the tauri app
```

The core crate (`windows/core`) holds everything that matters: frame encoding and decoding (including partial reads and garbage), the payload builder, the debounce (tested with tokio's paused clock), the connection manager (tested against an in-memory fake discord, including reconnects, bad ids and rate limits), presets, the game list and the app state. Only the named pipe connector is Windows specific, so that's the only part these tests can't run.

## Windows test checklist

1. **Install.** Install from the CI artifact. Launch mimic: the window opens and a tray icon appears.
2. **Connect.** With discord running, create a preset (New preset, pick a game). The side pill says "Not connected" until you go live, then "Connected as &lt;your name&gt;".
3. **Go live.** Fill in details and state, hit **Go live**, and look at your profile in discord (or the "Playing" line in the member list). Look from a second account to see buttons.
4. **Edit fields.** Change details, state, the timer (start now, custom offset, none), party size and a button while live. Rapid edits should become one update; the banner shows a rate limit countdown if you edit again within 15 seconds.
5. **Bad input.** Type `nope` as a button url: the field turns red with a message and nothing is sent.
6. **Switch games.** Make a second preset for a different game and click it in the sidebar, then **Go live**. Your profile should switch games within a second or two. Try the tray's **Quick switch** too.
7. **Clear.** **Stop & clear presence** removes it from your profile straight away.
8. **Tray.** Minimize: the window should vanish from the taskbar and the tray icon stays. Click the tray icon to bring it back. Close the window: mimic keeps running in the tray. **Quit mimic** from the tray menu really quits.
9. **Launch at login.** Turn it on in Settings (or the tray menu), sign out and back in. mimic should start hidden in the tray and put your last preset live again.
10. **Discord restarts.** Go live, then quit discord completely (right click its tray icon, Quit Discord). The banner should say discord isn't running and count down. Start discord again: mimic reconnects on its own and your presence comes back without clicking anything. Also try **Retry now** while discord is closed.
11. **Bad app id** (optional). If you can break an id on purpose, you should get a "Discord rejected this game's application id" banner. Normal list games shouldn't hit this.
12. **Game list.** Settings > **Refresh game list** should report a count close to what's bundled.
13. **Second launch.** Start mimic again while it's running: the running one comes to the front, no second copy.

**If something breaks, send back:**

- what you did and what you expected vs what happened (a screenshot helps)
- your Windows version, and whether discord is Stable, PTB or Canary
- the newest `.log` file from mimic's log folder, which is usually `%LOCALAPPDATA%\io.github.ag0nyy-ops.mimic\logs` (paste `%LOCALAPPDATA%\io.github.ag0nyy-ops.mimic` into Explorer's address bar and look for `logs`)
- `presets.json` from `%APPDATA%\io.github.ag0nyy-ops.mimic` if it's about presets (it only contains game names and the text you typed)

## The look

The interface follows a "classified file that watches you back" style: black and white only, blackletter titles, small wide-spaced mono labels, hairline borders, rubber stamps, film grain and a vignette, and an eye that follows your cursor and blinks. There is no colour anywhere, so status is carried by shape instead: a solid stamp or marker means fine, a dashed one means in progress, and an inverted (filled) one means a problem.

- Fonts are UnifrakturMaguntia, IBM Plex Mono and IBM Plex Sans, self-hosted in `windows/src/assets/fonts/`. They are under the SIL Open Font License 1.1, and the licence texts sit next to the files.
- Nothing is loaded from the internet by the interface. The only network request mimic ever makes is the game list refresh you start yourself.
- All motion is css. It does nothing under `prefers-reduced-motion`, and the grain and vignette are dropped under `prefers-contrast: more`.

## Project layout

```
shared/              platform independent spec and data
  README.md          the ipc protocol, written down
  preset.schema.json json schema of the saved preset file
  games.json         generated from discord's detectable applications
  scripts/           fetch-games.mjs regenerates games.json
windows/
  core/              rust crate with all the logic, tested anywhere (mimic-core)
  src-tauri/         the tauri shell: tray, window, autostart, commands
  src/               the frontend (vanilla typescript, vite), assets/ has the fonts and grain
.github/workflows/   build-windows.yml (tests + installers), release-windows.yml
```

## Why tauri and not egui

The things that make this app feel right are a live preview card, a distinctive look, and a polished form. All of that is much easier to get right in html and css than in egui's immediate mode widgets (the blackletter titles, the css-built eye and the scroll-driven effects below would be a fight). Tauri 2 also gives the tray icon, launch at login, single instance and nsis/msi bundling as ready made pieces. The cost is a small frontend (about 1,700 lines of plain typescript and 700 of css, no framework) and WebView2, which Windows already ships. The backend stays a thin shell over the core crate, which doesn't know tauri exists.

## Notes on choices that were left open

- Presets are saved on every edit (a moment after you stop typing), there's no save button.
- Going live with a preset replaces whatever was live.
- The last live preset is only restored on launch if you didn't hit Stop before quitting.
- Windows only: the app compiles on other platforms for type checking, but doesn't connect to discord there.
- Installers are unsigned.

## Disclaimer

mimic is for personal and novelty use. Faking a rich presence for a game you aren't playing may conflict with Discord's Terms of Service or Developer Terms, and using it could put your account at risk. You use it entirely at your own risk. It isn't affiliated with or endorsed by Discord.

## License

[MIT](LICENSE)
