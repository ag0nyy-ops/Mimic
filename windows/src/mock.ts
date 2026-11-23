// mimic · © 2026 Jay · https://github.com/ag0nyy-ops/mimic · MIT, see LICENSE
//
// Dev-only fake backend so the ui can be built and looked at in a normal
// browser (`npm run dev`, no tauri). Never part of a production build.

import type { Backend, Bootstrap, ConnectionState, Game, Live, Preset, SaveResult, Settings } from "./api";

export async function createMockBackend(): Promise<Backend> {
  const raw = (await import("../../shared/games.json")).default as Game[];
  const listeners = new Map<string, Set<(p: never) => void>>();
  const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((fn) => fn(payload as never));

  const params = new URLSearchParams(location.search);
  let conn: ConnectionState = { kind: "idle" };
  let live: Live | null = null;
  let settings: Settings = { closeToTray: true, minimizeToTray: true };
  let autostart = false;

  const demo = (id: string, name: string, extra: Partial<Preset>): Preset => ({
    id: crypto.randomUUID(),
    name,
    game: { id, name: raw.find((g) => g.id === id)?.name ?? name },
    details: "",
    state: "",
    assets: { largeImage: "", largeText: "", smallImage: "", smallText: "" },
    timer: { mode: "none" },
    party: null,
    buttons: [],
    ...extra,
  });
  const mc = raw.find((g) => g.name === "Minecraft") ?? raw[0];
  const presets: Preset[] =
    params.get("empty") === "1"
      ? []
      : [
          demo(mc.id, "Survival world", {
            details: "Building a base",
            state: "Hardcore, day 214",
            assets: { largeImage: "survival", largeText: "Overworld", smallImage: "pickaxe", smallText: "Mining" },
            timer: { mode: "offset", offsetSeconds: 5025 },
            party: { current: 2, max: 4 },
            buttons: [{ label: "Join my server", url: "https://example.com/join" }, { label: "", url: "" }],
          }),
          demo(raw.find((g) => g.name === "Terraria")?.id ?? raw[5].id, "Boss rush", { details: "Fighting the Moon Lord", timer: { mode: "now" } }),
          demo(raw.find((g) => g.name === "Counter-Strike 2")?.id ?? raw[9].id, "Comp grind", { details: "Competitive", state: "Dust II" }),
        ];

  const setConn = (c: ConnectionState) => {
    conn = c;
    emit("connection-state", c);
  };
  if (params.get("state") === "offline") conn = { kind: "discordNotRunning", retryInMs: 8000 };
  if (params.get("state") === "badapp") conn = { kind: "badAppId", message: "Invalid Client ID" };
  if (params.get("state") === "rate") conn = { kind: "rateLimited", retryInMs: 12000 };
  if (params.get("live") === "1" || params.get("state")) live = { presetId: presets[0]?.id ?? "", appliedAt: Math.floor(Date.now() / 1000) - 42 };
  if (params.get("live") === "1" && !params.get("state")) conn = { kind: "connected", username: "player" };

  const validate = (p: Preset): SaveResult["error"] => {
    for (let i = 0; i < p.buttons.length; i++) {
      const b = p.buttons[i];
      if (!b.label.trim() && !b.url.trim()) continue;
      if (!/^https?:\/\/[^\s/]+/i.test(b.url.trim())) {
        return { field: `buttons.${i}.url`, message: `button ${i + 1} url has to start with http:// or https://` };
      }
    }
    return null;
  };

  const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
    bootstrap: (): Bootstrap => ({
      version: "0.1.0-dev",
      presets,
      lastPresetId: live?.presetId ?? null,
      live,
      connection: conn,
      settings,
      autostart,
      games: { count: raw.length, source: "bundled" },
      warnings: [],
      dataDir: "C:\\Users\\you\\AppData\\Roaming\\io.github.ag0nyy-ops.mimic",
    }),
    search_games: ({ query, limit }) => {
      const q = String(query).toLowerCase().trim();
      const out = q ? raw.filter((g) => g.name.toLowerCase().includes(q)) : raw;
      return out.slice(0, Number(limit ?? 50));
    },
    refresh_games: async () => {
      await new Promise((r) => setTimeout(r, 900));
      return { count: raw.length, source: "downloaded" };
    },
    create_preset: ({ game }) => {
      const p = demo((game as Game).id, (game as Game).name, {});
      presets.push(p);
      return p;
    },
    save_preset: ({ preset }): SaveResult => {
      const p = preset as Preset;
      const i = presets.findIndex((x) => x.id === p.id);
      if (i >= 0) presets[i] = p;
      return { preset: p, error: validate(p), live };
    },
    duplicate_preset: ({ id }) => {
      const i = presets.findIndex((x) => x.id === id);
      const copy = { ...structuredClone(presets[i]), id: crypto.randomUUID(), name: presets[i].name + " copy" };
      presets.splice(i + 1, 0, copy);
      return copy;
    },
    delete_preset: ({ id }) => {
      const i = presets.findIndex((x) => x.id === id);
      if (i >= 0) presets.splice(i, 1);
      if (live?.presetId === id) {
        live = null;
        emit("live-changed", null);
        setConn({ kind: "idle" });
      }
    },
    go_live: ({ id }) => {
      const err = validate(presets.find((p) => p.id === id)!);
      if (err) throw err;
      live = { presetId: String(id), appliedAt: Math.floor(Date.now() / 1000) };
      setConn({ kind: "connecting" });
      setTimeout(() => setConn({ kind: "connected", username: "player" }), 700);
      return live;
    },
    stop_live: () => {
      live = null;
      emit("live-changed", null);
      setConn({ kind: "idle" });
    },
    retry_connection: () => setConn({ kind: "connecting" }),
    save_settings: ({ settings: s }) => void (settings = s as Settings),
    set_autostart: ({ enabled }) => (autostart = Boolean(enabled)),
  };

  return {
    call: async <T>(cmd: string, args: Record<string, unknown> = {}) => {
      const fn = handlers[cmd];
      if (!fn) throw new Error(`mock: no handler for ${cmd}`);
      return (await fn(args)) as T;
    },
    on: async (event, handler) => {
      const set = listeners.get(event) ?? new Set();
      set.add(handler as (p: never) => void);
      listeners.set(event, set);
      return () => void set.delete(handler as (p: never) => void);
    },
  };
}
