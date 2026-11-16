/**
 * The iii mode: Claude Code has no plugin-defined permission modes, so the
 * mode is two plugin parts layered on whichever built-in mode is active:
 *
 *   - the `iii` agent (`iii-plugin/agents/iii.md`), which a session runs as
 *     with `--agent iii:iii` (the worker passes it; standalone users opt in);
 *   - a PreToolUse hook that pre-approves the plugin's read-only tools.
 *
 * `iii_call`, `iii_observe` and `iii_secret_store` are never pre-approved: they
 * can do anything on the bus (or write the vault), so they stay with the
 * session's permission mode and, in a headless turn, the worker's approval
 * gate.
 */

import { TOOL_PREFIX } from './commands.js';

/** `--agent` value: plugin `iii`, agent `iii`. */
export const III_AGENT = 'iii:iii';

export const APPROVED_TOOLS = [
  'iii_status',
  'iii_list_agents',
  'iii_get_agent',
  'iii_list_skills',
  'iii_get_skill',
  'iii_observations_poll',
  'iii_observations_sse',
  'iii_observers',
  'iii_unobserve',
];

/** Calling and observing can do anything on the bus; storing writes the vault. */
export const NEVER_APPROVED_TOOLS = ['iii_call', 'iii_observe', 'iii_secret_store'];

/**
 * What a headless turn may run without a prompt nobody is there to answer:
 * the read-only tools plus calling and observing. rbac-proxy decides what
 * those reach, which is narrower than the `Bash(iii *)` a turn already has.
 * Storing a secret stays with the approval gate.
 */
export const HEADLESS_TOOLS = [...APPROVED_TOOLS, 'iii_call', 'iii_observe'].map(
  (t) => `${TOOL_PREFIX}${t}`,
);

const DECISION = {
  hookSpecificOutput: {
    hookEventName: 'PreToolUse',
    permissionDecision: 'allow',
    permissionDecisionReason: 'iii plugin: read-only engine tool',
  },
};

export type HookEntry = { matcher: string; hooks: { type: 'command'; command: string }[] };

/** The PreToolUse entry that allows the read-only tools, and nothing else. */
export function approveHookEntry(): HookEntry {
  return {
    matcher: `^${TOOL_PREFIX}(${APPROVED_TOOLS.join('|')})$`,
    hooks: [{ type: 'command', command: `printf '%s\\n' '${JSON.stringify(DECISION)}'` }],
  };
}

/**
 * The iii tools a headless turn allows. With the approval gate on, calling
 * and observing are left to the gate (an allow rule would skip it).
 */
export function headlessTools(approvalGate: boolean): string[] {
  return approvalGate ? APPROVED_TOOLS.map((t) => `${TOOL_PREFIX}${t}`) : HEADLESS_TOOLS;
}
