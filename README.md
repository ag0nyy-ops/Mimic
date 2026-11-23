> **License:** source-available, view and learn only. Not open source. See [LICENSE](LICENSE) before cloning, forking or using this code.

# Mimic

Mimic a game on your discord rich presence, so you can be the best larp in the world.

![Mimic's main window: a preset being edited next to a live preview card](docs/screenshots/main.webp)

Pick any game from discord's own list, write what your profile should say, and go live. A preview card updates as you type.

**Windows only for now. macOS and Linux versions are still to come.**

<p>
  <img src="docs/screenshots/picker.webp" alt="The searchable game picker" width="49%">
  <img src="docs/screenshots/no-signal.webp" alt="The 'no signal' state when discord isn't running" width="49%">
</p>

<sub>Screenshots show the real interface with sample presets.</sub>

## Features

- Searchable picker over about 25,000 games, taken from discord's public list (ids are never typed in by hand)
- Details, state, large and small image keys, elapsed timer, party size and up to two buttons
- Saved presets, quick switch from the tray, and your last preset comes back on launch
- Clear error states: discord not running, bad application id, rate limited, invalid button url
- Tray icon, close to tray, launch at login

## Install

1. Download the `.exe` or `.msi` from the [Releases](../../releases) page. Want the latest unreleased build? Open the newest green **build-windows** run under **Actions** and download its `mimic-windows-installers` artifact.
2. Run it. The installers aren't code signed, so SmartScreen warns: click **More info**, then **Run anyway**.

Start the **discord desktop app** first (the browser version can't work), then open Mimic, make a preset and hit **Go live**.

## How it works

Discord's desktop app listens on a local pipe. Mimic connects to it, says which game it is (that game's application id), and sends the status you wrote. Switching games opens a fresh connection with the new id; stopping closes it, which makes discord drop the presence. Updates are batched to stay inside discord's rate limit of about one per 15 seconds. The protocol is written up in [`shared/README.md`](shared/README.md).

## Good to know

- **Image keys** only show if the game's discord application owns an asset with that exact name, which usually isn't the case for keys you make up. Whether a full image url works as a key is untested.
- **Buttons** are never visible on your own profile, only to other people.
- **The elapsed timer** keeps running while you edit, and only restarts when you change the timer setting.

## Build from source

You need [Rust](https://rustup.rs) with the MSVC toolchain (Visual Studio Build Tools, "Desktop development with C++"), [Node.js](https://nodejs.org) 22+ and WebView2 (already on current Windows). Mimic is a [Tauri 2](https://tauri.app) app, chosen over egui because the live preview and the look are far easier to get right in html and css.

From `windows/`:

```sh
npm install
npm run tauri dev      # run it
npm run tauri build    # installers land in target/release/bundle/
cargo test             # the core crate's tests, runs on any os
```

`npm run dev` opens just the interface in a browser with a fake backend, handy for styling work.

## Testing a build

Try: connect and go live, edit every field, switch games (sidebar and tray), stop, minimize and close to tray, launch at login, and quit discord while live (Mimic should reconnect and bring the presence back by itself).

If something breaks, send what you did, your Windows version, whether discord is Stable, PTB or Canary, and the newest `.log` in `%LOCALAPPDATA%\io.github.ag0nyy-ops.mimic\logs`.

## Layout

```
shared/      protocol notes, preset schema and the game list (made by scripts/fetch-games.mjs)
windows/
  core/      all the logic, tested anywhere
  src-tauri/ the Tauri shell: tray, window, autostart
  src/       the interface (TypeScript, no framework)
```

The look is a black and white "classified file" style. Its fonts (UnifrakturMaguntia, IBM Plex Mono and Sans) are bundled under the SIL Open Font License, with the licence texts in `windows/src/assets/fonts/`, and the interface loads nothing from the internet.

## Disclaimer

For personal and novelty use. Faking a rich presence may conflict with Discord's Terms of Service and could put your account at risk. Use it at your own risk. Not affiliated with or endorsed by Discord.

## License

Source-available, view and learn only. See [LICENSE](LICENSE).

Not covered by it: `shared/games.json` is a list of game names and application IDs that come from Discord, and the bundled fonts stay under the SIL Open Font License (see above).
