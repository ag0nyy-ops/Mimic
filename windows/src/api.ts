// Types mirror the rust side (camelCase json) and thin wrappers around invoke().

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Game {
  id: string;
  name: string;
}

export type Timer =
  | { mode: "none" }
  | { mode: "now" }
  | { mode: "offset"; offsetSeconds: number };

export interface ButtonInput {
  label: string;
  url: string;
}

export interface Preset {
  id: string;
  name: string;
  game: Game;
  details: string;
  state: string;
  assets: {
    largeImage: string;
    largeText: string;
    smallImage: string;
    smallText: string;
  };
  timer: Timer;
  party: { current: number; max: number } | null;
  buttons: ButtonInput[];
}

export type ConnectionState =
  | { kind: "idle" }
  | { kind: "discordNotRunning"; retryInMs: number }
  | { kind: "connecting" }
  | { kind: "connected"; username: string | null }
  | { kind: "badAppId"; message: string }
  | { kind: "rateLimited"; retryInMs: number }
  | { kind: "error"; message: string };

export interface Settings {
  closeToTray: boolean;
  minimizeToTray: boolean;
}

export interface Live {
  presetId: string;
  appliedAt: number;
}

export interface GamesInfo {
  count: number;
  source: "bundled" | "downloaded";
}

export interface CmdError {
  field: string | null;
  message: string;
}

export interface SaveResult {
  preset: Preset;
  error: CmdError | null;
  live: Live | null;
}

export interface Bootstrap {
  version: string;
  presets: Preset[];
  lastPresetId: string | null;
  live: Live | null;
  connection: ConnectionState;
  settings: Settings;
  autostart: boolean;
  games: GamesInfo;
  warnings: string[];
  dataDir: string;
}

export interface Backend {
  call<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn>;
}

const tauriBackend: Backend = {
  call: (cmd, args) => invoke(cmd, args),
  on: (event, handler) => listen<never>(event, (e) => handler(e.payload as never)),
};

let backend: Backend = tauriBackend;

/** In a plain browser (`npm run dev` without tauri) fall back to an in-memory fake. */
export async function initBackend(): Promise<void> {
  const inTauri = typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== "undefined";
  if (!inTauri && import.meta.env.DEV) {
    const { createMockBackend } = await import("./mock");
    backend = await createMockBackend();
  }
}

export const api = {
  bootstrap: () => backend.call<Bootstrap>("bootstrap"),
  searchGames: (query: string, limit = 50) => backend.call<Game[]>("search_games", { query, limit }),
  refreshGames: () => backend.call<GamesInfo>("refresh_games"),
  createPreset: (game: Game) => backend.call<Preset>("create_preset", { game }),
  savePreset: (preset: Preset) => backend.call<SaveResult>("save_preset", { preset }),
  duplicatePreset: (id: string) => backend.call<Preset>("duplicate_preset", { id }),
  deletePreset: (id: string) => backend.call<void>("delete_preset", { id }),
  goLive: (id: string) => backend.call<Live>("go_live", { id }),
  stopLive: () => backend.call<void>("stop_live"),
  retryConnection: () => backend.call<void>("retry_connection"),
  saveSettings: (settings: Settings) => backend.call<void>("save_settings", { settings }),
  setAutostart: (enabled: boolean) => backend.call<boolean>("set_autostart", { enabled }),
  onConnection: (handler: (s: ConnectionState) => void) => backend.on<ConnectionState>("connection-state", handler),
  onLive: (handler: (l: Live | null) => void) => backend.on<Live | null>("live-changed", handler),
};

/** Errors from commands arrive as the serialized CmdError, or a plain string. */
export function toCmdError(e: unknown): CmdError {
  if (e && typeof e === "object" && "message" in e) {
    const { field, message } = e as { field?: string | null; message: unknown };
    return { field: field ?? null, message: String(message) };
  }
  return { field: null, message: String(e) };
}
