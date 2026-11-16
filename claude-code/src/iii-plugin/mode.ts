/**
 * The `iii` agent and the read-only approval hook. Neither is a Claude Code
 * permission mode: those are built in (default, accept edits, plan, auto,
 * bypass) and a plugin cannot add one. The pieces:
 *
 *   - the `iii` agent (`iii-plugin/agents/iii.md`), built from the default-mcp
 *     profile, which a session runs as only with `--agent iii:iii` (it then
 *     replaces Claude Code's system prompt);
 *   - a PreToolUse hook that pre-approves the plugin's read-only tools in
 *     whatever permission mode is active.
 *
 * `iii_call`, `iii_observe` and `iii_secret_store` are never pre-approved by
 * the hook: they stay with the session's permission mode and, in a headless
 * turn, the worker's allow list and approval gate.
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
