import { bridge } from "./bridge";
import type { Workspace } from "./types";

export type { Workspace } from "./types";

const stateKey = (sessionId: string) => `session/${sessionId}/workspace`;

/**
 * Load the workspace for a session. Returns null when nothing has been saved
 * yet. Returns null on bridge errors too — the workspace is advisory and
 * should never block UI rendering.
 */
export async function loadWorkspace(sessionId: string): Promise<Workspace | null> {
  try {
    const value = await bridge<Workspace | null>("state::get", {
      scope: "agent",
      key: stateKey(sessionId),
    });
    if (value && typeof value.cwd === "string" && typeof value.set_at === "number") {
      return value;
    }
    return null;
  } catch {
    return null;
  }
}

export async function saveWorkspace(sessionId: string, cwd: string): Promise<void> {
  const value: Workspace = { cwd, set_at: Date.now() };
  await bridge<void>("state::set", {
    scope: "agent",
    key: stateKey(sessionId),
    value,
  });
}
