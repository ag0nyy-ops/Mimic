import "./style.css";

import { api, initBackend, toCmdError, type CmdError, type ConnectionState, type GamesInfo, type Live, type Preset, type Settings } from "./api";
import { h, icon, clear } from "./dom";
import { Editor, mapErrorField, padButtons, serializeForSave } from "./editor";
import { pickGame } from "./picker";
import { PreviewCard } from "./preview";

// ---------------------------------------------------------------- state

const app = {
  version: "",
  dataDir: "",
  presets: [] as Preset[],
  selectedId: null as string | null,
  live: null as Live | null,
  conn: { kind: "idle" } as ConnectionState,
  connAt: performance.now(),
  settings: { closeToTray: true, minimizeToTray: true } as Settings,
  autostart: false,
  games: { count: 0, source: "bundled" } as GamesInfo,
  draft: null as Preset | null,
  rowMap: [] as number[],
  /** problem with the selected preset that stops it reaching discord */
  problem: null as CmdError | null,
  /** the elapsed timer anchor while the selected preset isn't live */
  previewBase: Math.floor(Date.now() / 1000),
};

const root = document.getElementById("app")!;
const preview = new PreviewCard();

const selectedPreset = () => app.presets.find((p) => p.id === app.selectedId) ?? null;
const isLive = (id: string | null) => id !== null && app.live?.presetId === id;

// ---------------------------------------------------------------- toasts

const toastHost = h("div", { class: "toasts" });
function toast(message: string, tone: "info" | "bad" = "info"): void {
  const el = h("div", { class: `toast ${tone}` }, message);
  toastHost.append(el);
  window.setTimeout(() => el.classList.add("out"), 4200);
  window.setTimeout(() => el.remove(), 4600);
}

// ---------------------------------------------------------------- saving

let saveTimer: number | undefined;
let saveChain: Promise<void> = Promise.resolve();

function scheduleSave(): void {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void flushSave(), 350);
}

function flushSave(): Promise<void> {
  window.clearTimeout(saveTimer);
  const draft = app.draft;
  if (!draft) return saveChain;
  const { preset, rowMap } = serializeForSave(draft);
  saveChain = saveChain.then(async () => {
    try {
      const result = await api.savePreset(preset);
      if (app.draft?.id !== preset.id) return; // user moved on while this was in flight
      app.rowMap = rowMap;
      if (result.live) app.live = result.live;
      setProblem(result.error);
      renderSide();
      renderSidebar();
    } catch (e) {
      toast(`Couldn't save: ${toCmdError(e).message}`, "bad");
    }
  });
  return saveChain;
}

function setProblem(err: CmdError | null): void {
  app.problem = err;
  const field = err ? mapErrorField(err.field, app.rowMap) : null;
  editor.setError(field, err?.message ?? null);
}

// ---------------------------------------------------------------- editor

const editor = new Editor({
  onChange: (draft) => {
    const i = app.presets.findIndex((p) => p.id === draft.id);
    if (i >= 0) app.presets[i] = draft;
    renderPreview();
    renderSidebar();
    scheduleSave();
  },
  onChangeGame: async () => {
    const game = await pickGame({ title: "Change game", total: app.games.count, search: (q) => api.searchGames(q) });
    if (!game || !app.draft) return;
    const wasName = app.draft.name === app.draft.game.name;
    app.draft.game = game;
    if (wasName) app.draft.name = game.name;
    editor.load(app.draft);
    renderPreview();
    renderSidebar();
    scheduleSave();
  },
  onDuplicate: async () => {
    if (!app.selectedId) return;
    await flushSave();
    try {
      const copy = await api.duplicatePreset(app.selectedId);
      await reloadPresets();
      await select(copy.id);
    } catch (e) {
      toast(toCmdError(e).message, "bad");
    }
  },
  onDelete: async () => {
    const preset = selectedPreset();
    if (!preset) return;
    if (!window.confirm(`Delete “${preset.name}”?${isLive(preset.id) ? " It is live right now, so that stops your presence." : ""}`)) return;
    await api.deletePreset(preset.id);
    if (isLive(preset.id)) app.live = null;
    app.draft = null;
    await reloadPresets();
    await select(app.presets[0]?.id ?? null);
  },
});

async function reloadPresets(): Promise<void> {
  const boot = await api.bootstrap();
  app.presets = boot.presets;
  app.live = boot.live;
}

async function select(id: string | null): Promise<void> {
  await flushSave();
  app.selectedId = id;
  try {
    if (id) localStorage.setItem("mimic.selected", id);
  } catch {
    /* storage can be unavailable, the selection just won't be remembered */
  }
  const preset = selectedPreset();
  app.draft = preset ? { ...structuredClone(preset), buttons: padButtons(preset.buttons) } : null;
  app.problem = null;
  app.previewBase = Math.floor(Date.now() / 1000);
  if (app.draft) editor.load(app.draft);
  renderAll();
  if (app.draft) scheduleSave(); // validates the freshly loaded preset so errors show up straight away
}

// ---------------------------------------------------------------- going live

async function goLive(): Promise<void> {
  const preset = selectedPreset();
  if (!preset) return;
  await flushSave();
  try {
    app.live = await api.goLive(preset.id);
    setProblem(null);
  } catch (e) {
    const err = toCmdError(e);
    setProblem(err);
    toast(`Not sent to discord: ${err.message}`, "bad");
  }
  renderAll();
}

async function stopLive(): Promise<void> {
  await api.stopLive();
  app.live = null;
  renderAll();
}

// ---------------------------------------------------------------- sidebar

const sidebarList = h("div", { class: "preset-list" });
const sidebarPill = h("div", { class: "pill" });

function pillFor(conn: ConnectionState): { tone: string; text: string } {
  switch (conn.kind) {
    case "idle":
      return { tone: "off", text: "Not connected" };
    case "connecting":
      return { tone: "warn", text: "Connecting…" };
    case "connected":
      return { tone: "ok", text: conn.username ? `Connected as ${conn.username}` : "Connected" };
    case "discordNotRunning":
      return { tone: "bad", text: "Discord not running" };
    case "badAppId":
      return { tone: "bad", text: "Bad app id" };
    case "rateLimited":
      return { tone: "warn", text: "Rate limited" };
    case "error":
      return { tone: "bad", text: "Error" };
  }
}

function renderSidebar(): void {
  const pill = pillFor(app.conn);
  sidebarPill.className = `pill ${pill.tone}`;
  sidebarPill.replaceChildren(h("span", { class: "dot" }), pill.text);

  sidebarList.replaceChildren(
    ...app.presets.map((p) => {
      const live = isLive(p.id);
      return h(
        "div",
        {
          class: "preset-item" + (p.id === app.selectedId ? " selected" : "") + (live ? " live" : ""),
          role: "button",
          tabIndex: 0,
          onclick: () => void select(p.id),
          onkeydown: (e: KeyboardEvent) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              void select(p.id);
            }
          },
        },
        h("div", { class: "avatar" }, (p.game.name.trim()[0] ?? "?").toUpperCase()),
        h("div", { class: "preset-text" }, h("div", { class: "preset-name" }, p.name || p.game.name), h("div", { class: "preset-game" }, p.game.name)),
        live ? h("span", { class: "live-badge" }, "LIVE") : null,
        h(
          "button",
          {
            class: "quick" + (live ? " stop" : ""),
            type: "button",
            title: live ? "Stop" : "Go live",
            onclick: (e: MouseEvent) => {
              e.stopPropagation();
              void (live ? stopLive() : select(p.id).then(goLive));
            },
          },
          icon(live ? "stop" : "play", 14),
        ),
      );
    }),
  );
}

// ---------------------------------------------------------------- right pane

const banner = h("div", { class: "banner" });
const actions = h("div", { class: "actions" });
const previewHost = h("div", { class: "preview-host" });
previewHost.append(preview.el);

interface BannerSpec {
  tone: "info" | "ok" | "warn" | "bad";
  title: string;
  text: string;
  retry?: boolean;
}

function secondsLeft(retryInMs: number): number {
  return Math.max(0, Math.ceil((retryInMs - (performance.now() - app.connAt)) / 1000));
}

function bannerSpec(): BannerSpec {
  const live = app.live ? app.presets.find((p) => p.id === app.live!.presetId) : null;
  const c = app.conn;
  switch (c.kind) {
    case "discordNotRunning":
      return {
        tone: "bad",
        title: "Discord isn't running",
        text: `mimic can't find discord on this pc. Start the discord desktop app (the browser version won't work) and mimic connects by itself. Next try in ${secondsLeft(c.retryInMs)}s.`,
        retry: true,
      };
    case "badAppId":
      return {
        tone: "bad",
        title: "Discord rejected this game's application id",
        text: `${c.message || "Invalid client id"}. Pick a different game, or retry if you think it was a one-off.`,
        retry: true,
      };
    case "rateLimited": {
      const s = secondsLeft(c.retryInMs);
      return {
        tone: "warn",
        title: "Rate limited",
        text: `Discord only takes about one presence update every 15 seconds. Your latest edit goes out in ${s}s, and anything you change before then is folded into it.`,
      };
    }
    case "error":
      return { tone: "bad", title: "Discord reported a problem", text: c.message, retry: true };
    case "connecting":
      return { tone: "info", title: "Connecting to discord…", text: "Looking for discord's local ipc pipe." };
    case "connected":
      return {
        tone: "ok",
        title: live ? `Live as ${live.game.name}` : "Connected to discord",
        text: live ? "Edits to this preset are sent automatically." : "Nothing is being shown right now.",
      };
    case "idle":
      return app.presets.length
        ? { tone: "info", title: "Not live", text: "Pick a preset and hit Go live. Nothing is shown on your profile until you do." }
        : { tone: "info", title: "Not live", text: "Create a preset to get started." };
  }
}

function renderBanner(): void {
  const spec = app.problem
    ? ({ tone: "warn", title: "Not sent to discord", text: app.problem.message } satisfies BannerSpec)
    : bannerSpec();
  const iconName = spec.tone === "ok" ? "check" : spec.tone === "info" ? "info" : "alert";
  banner.className = `banner ${spec.tone}`;
  banner.replaceChildren(
    h("div", { class: "banner-icon" }, icon(iconName, 18)),
    h(
      "div",
      { class: "banner-body" },
      h("div", { class: "banner-title" }, spec.title),
      h("div", { class: "banner-text" }, spec.text),
      spec.retry && !app.problem
        ? h("button", { class: "btn small", type: "button", onclick: () => void api.retryConnection() }, icon("refresh", 14), "Retry now")
        : null,
    ),
  );
}

function renderActions(): void {
  const preset = selectedPreset();
  if (!preset) {
    clear(actions);
    return;
  }
  const live = isLive(preset.id);
  actions.replaceChildren(
    live
      ? h("button", { class: "btn danger big", type: "button", onclick: () => void stopLive() }, icon("stop", 16), "Stop & clear presence")
      : h("button", { class: "btn primary big", type: "button", onclick: () => void goLive() }, icon("play", 16), "Go live"),
    h(
      "p",
      { class: "hint" },
      live
        ? "Stopping closes the connection, which makes discord drop your presence right away."
        : app.live
          ? "Going live here replaces the preset that is live right now."
          : "Mimic only shows a presence while it's live.",
    ),
  );
}

function renderPreview(): void {
  const preset = app.draft;
  if (!preset) return;
  const base = isLive(preset.id) ? app.live!.appliedAt : app.previewBase;
  preview.update(preset, base);
}

function renderSide(): void {
  renderBanner();
  renderActions();
  renderPreview();
}

// ---------------------------------------------------------------- settings

function openSettings(): void {
  const close = () => {
    document.removeEventListener("keydown", onKey, true);
    overlay.remove();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  };
  document.addEventListener("keydown", onKey, true);

  const toggle = (label: string, hint: string, checked: boolean, onChange: (v: boolean, input: HTMLInputElement) => void) => {
    const input = h("input", { type: "checkbox", checked });
    input.addEventListener("change", () => onChange(input.checked, input));
    return h("label", { class: "setting" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label" }, label), h("div", { class: "hint" }, hint)), input, h("span", { class: "switch" }));
  };

  const gamesLine = h("div", { class: "hint" });
  const paintGames = () => {
    gamesLine.textContent = `${app.games.count.toLocaleString()} games · ${app.games.source === "bundled" ? "the list that shipped with this version" : "downloaded from discord"}`;
  };
  paintGames();
  const refreshBtn = h("button", { class: "btn", type: "button" }, icon("refresh", 14), "Refresh game list");
  refreshBtn.addEventListener("click", async () => {
    refreshBtn.disabled = true;
    refreshBtn.classList.add("busy");
    try {
      app.games = await api.refreshGames();
      paintGames();
      toast(`Game list updated: ${app.games.count.toLocaleString()} games`);
    } catch (e) {
      toast(toCmdError(e).message, "bad");
    } finally {
      refreshBtn.disabled = false;
      refreshBtn.classList.remove("busy");
    }
  });

  const saveSettings = () => void api.saveSettings(app.settings);

  const overlay = h(
    "div",
    {
      class: "overlay",
      onmousedown: (e: MouseEvent) => {
        if (e.target === overlay) close();
      },
    },
    h(
      "div",
      { class: "modal settings", role: "dialog", "aria-label": "Settings" },
      h("div", { class: "modal-head" }, h("h2", null, "Settings"), h("button", { class: "icon-btn", type: "button", title: "Close", onclick: close }, icon("x"))),
      h(
        "div",
        { class: "settings-body" },
        toggle("Launch at login", "Start mimic hidden in the tray when you sign in, and bring back your last preset.", app.autostart, async (v, input) => {
          try {
            app.autostart = await api.setAutostart(v);
          } catch (e) {
            input.checked = !v;
            toast(toCmdError(e).message, "bad");
          }
        }),
        toggle("Minimize to tray", "Minimizing hides the window instead of leaving it on the taskbar.", app.settings.minimizeToTray, (v) => {
          app.settings.minimizeToTray = v;
          saveSettings();
        }),
        toggle("Close to tray", "The close button hides mimic and keeps your presence running. Quit from the tray icon.", app.settings.closeToTray, (v) => {
          app.settings.closeToTray = v;
          saveSettings();
        }),
        h("div", { class: "setting-block" }, h("div", { class: "setting-label" }, "Game list"), gamesLine, refreshBtn),
        h(
          "div",
          { class: "about" },
          h("div", null, `mimic ${app.version}`),
          h("div", { class: "hint" }, `Data folder: ${app.dataDir}`),
          h("div", { class: "hint" }, "For personal and novelty use. Faking a rich presence may go against discord's terms of service, so use it at your own risk."),
        ),
      ),
    ),
  );
  document.body.append(overlay);
}

// ---------------------------------------------------------------- layout

const emptyState = h(
  "div",
  { class: "empty" },
  h("div", { class: "empty-art" }, icon("gamepad", 44)),
  h("h2", null, "Be whatever you want to be playing"),
  h("p", null, "Pick any game from discord's list and set exactly what your profile says: details, state, timer, party size and buttons."),
  h("button", { class: "btn primary big", type: "button", onclick: () => void newPreset() }, icon("plus", 16), "Create your first preset"),
);

async function newPreset(): Promise<void> {
  const game = await pickGame({ title: "Pick a game", total: app.games.count, search: (q) => api.searchGames(q) });
  if (!game) return;
  try {
    const preset = await api.createPreset(game);
    await reloadPresets();
    await select(preset.id);
  } catch (e) {
    toast(toCmdError(e).message, "bad");
  }
}

const sidebar = h(
  "aside",
  { class: "sidebar" },
  h("div", { class: "brand" }, h("div", { class: "logo" }, icon("play", 14)), h("span", null, "mimic")),
  sidebarPill,
  h("div", { class: "side-label" }, "Presets"),
  sidebarList,
  h("button", { class: "btn new", type: "button", onclick: () => void newPreset() }, icon("plus", 16), "New preset"),
  h("button", { class: "btn ghost settings-btn", type: "button", onclick: openSettings }, icon("sliders", 16), "Settings"),
);

const editorPane = h("main", { class: "editor-pane" }, editor.head, h("div", { class: "editor-scroll" }, editor.body));
const sidePane = h(
  "section",
  { class: "side-pane" },
  banner,
  h("div", { class: "side-label" }, "Live preview"),
  previewHost,
  actions,
);
const workspace = h("div", { class: "workspace" }, editorPane, sidePane);
const main = h("div", { class: "main" }, workspace, emptyState);

function renderAll(): void {
  const hasSelection = app.draft !== null;
  workspace.hidden = !hasSelection;
  emptyState.hidden = hasSelection;
  renderSidebar();
  renderSide();
}

// ---------------------------------------------------------------- boot

async function boot(): Promise<void> {
  await initBackend();
  root.append(h("div", { class: "app" }, sidebar, main), toastHost);

  const boot = await api.bootstrap();
  Object.assign(app, {
    version: boot.version,
    dataDir: boot.dataDir,
    presets: boot.presets,
    live: boot.live,
    conn: boot.connection,
    connAt: performance.now(),
    settings: boot.settings,
    autostart: boot.autostart,
    games: boot.games,
  });
  for (const w of boot.warnings) toast(w, "bad");

  await api.onConnection((state) => {
    app.conn = state;
    app.connAt = performance.now();
    renderSidebar();
    renderBanner();
  });
  await api.onLive((live) => {
    app.live = live;
    renderAll();
  });
  // keep the countdowns in the banner ticking
  window.setInterval(() => {
    if (app.conn.kind === "discordNotRunning" || app.conn.kind === "rateLimited") renderBanner();
  }, 500);

  let remembered: string | null = null;
  try {
    remembered = localStorage.getItem("mimic.selected");
  } catch {
    /* ignore */
  }
  const ids = new Set(boot.presets.map((p) => p.id));
  const first =
    [boot.live?.presetId, boot.lastPresetId, remembered].find((id): id is string => !!id && ids.has(id)) ?? boot.presets[0]?.id ?? null;
  await select(first);
}

void boot();
