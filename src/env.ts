// Minimal .env loader — no dependency. Reads KEY=VALUE lines from ./.env if
// present; real environment variables win over file entries.
import { readFileSync } from "fs";

try {
  const lines = readFileSync(new URL("../.env", import.meta.url), "utf8").split("\n");
  for (const line of lines) {
    const m = line.match(/^([A-Z0-9_]+)=(.*)$/);
    if (m && process.env[m[1]] === undefined) process.env[m[1]] = m[2];
  }
} catch {
  // no .env file — fine, rely on the environment
}

export function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) {
    console.error(`Missing ${name} — set it in the environment or in .env (see .env.example)`);
    process.exit(1);
  }
  return v;
}
