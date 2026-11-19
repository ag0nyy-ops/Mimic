#!/usr/bin/env node
// Regenerates shared/games.json from discord's public "detectable applications"
// endpoint. Keeps only id + name, drops duplicates, sorts by name.
//
// usage: node shared/scripts/fetch-games.mjs [output-path]
//
// Exits non-zero (and leaves the existing file untouched) if the request fails
// or the response doesn't look right. It never invents data.

import { writeFile, rename } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const ENDPOINT = "https://discord.com/api/v9/applications/detectable";
const here = path.dirname(fileURLToPath(import.meta.url));
const out = path.resolve(process.argv[2] ?? path.join(here, "..", "games.json"));

// Discord application ids are snowflakes.
const SNOWFLAKE = /^[0-9]{15,25}$/;

async function main() {
  const res = await fetch(ENDPOINT, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`discord answered ${res.status} ${res.statusText}`);

  const raw = await res.json();
  if (!Array.isArray(raw)) throw new Error("expected a json array from discord");

  const seen = new Set();
  const games = [];
  for (const app of raw) {
    const id = typeof app?.id === "string" ? app.id : null;
    const name = typeof app?.name === "string" ? app.name.trim() : "";
    if (!id || !SNOWFLAKE.test(id) || !name || seen.has(id)) continue;
    seen.add(id);
    games.push({ id, name });
  }
  if (games.length < 100) {
    throw new Error(`only ${games.length} usable entries, refusing to overwrite ${out}`);
  }

  games.sort((a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }) || a.id.localeCompare(b.id));

  const tmp = `${out}.tmp`;
  await writeFile(tmp, JSON.stringify(games) + "\n");
  await rename(tmp, out);
  console.log(`wrote ${games.length} games to ${out} (from ${raw.length} entries)`);
}

main().catch((err) => {
  console.error(`fetch-games failed: ${err.message}`);
  process.exit(1);
});
