/**
 * The pi extension this worker installs into the workspace.
 *
 * pi has extensions where Claude Code has shell hooks, so this is pi's half
 * of the same contract: one flat event per lifecycle callback, posted to
 * `pi::terminal::activity` with the `iii` CLI. The bus is the transport because the
 * terminal host is not necessarily this worker's host, and `pi.exec` keeps the
 * payload an argument rather than a shell string.
 *
 * Shipped as text (not compiled): pi loads and type-checks the file itself,
 * and it must live in the workspace where pi discovers it
 * (`.pi/extensions/*.ts`).
 */

export const EXTENSION_PATH = '.pi/extensions/iii-activity.ts';

export function extensionSource(cli: string): string {
  return `// Written by the pi iii worker on every boot; edits are lost.
//
// Reports this session's turns and tool calls to the iii engine, where the
// pi worker turns them into AgentEvent frames on agent::events — the same
// shape every other agent worker on the bus emits.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const CLI = ${JSON.stringify(cli)};
// One pi process is one session, and pi's own id is not needed for the worker
// to keep a turn and its tool calls together.
const SESSION_ID = \`pi-\${process.pid}-\${Date.now().toString(36)}\`;

// pi's SDK loads this file for a HEADLESS turn too, because the worker runs
// that turn in this workspace and \`createAgentSession\` discovers
// \`.pi/extensions/\` from its cwd. In that case the worker is already
// reporting the turn itself, under the iii session id the caller was given —
// so reporting again from here would put the same run on \`agent::events\`
// twice, under two different session ids. The worker marks its own process at
// boot; finding that mark means "not mine to report".
const IN_WORKER_PROCESS = Boolean((globalThis as { __iiiPiWorker?: boolean }).__iiiPiWorker);

export default function (pi: ExtensionAPI) {
  if (IN_WORKER_PROCESS) return;
  const post = (event: Record<string, unknown>): void => {
    void pi
      .exec(CLI, [
        "trigger",
        "pi::terminal::activity",
        "--json",
        JSON.stringify({ session_id: SESSION_ID, ...event }),
        "--timeout-ms",
        "3000",
      ], { timeout: 5000 })
      .catch(() => undefined);
  };

  pi.on("session_start", (_event: unknown, ctx: { cwd?: string }) => {
    post({ event: "session_start", cwd: ctx?.cwd });
  });

  pi.on("session_shutdown", () => {
    post({ event: "session_end" });
  });

  // One agent run = one prompt answered, however many model turns that takes.
  // That is the unit worth being a turn on the stream.
  pi.on("before_agent_start", (event: { prompt?: string }) => {
    post({ event: "agent_start", prompt: String(event?.prompt ?? "") });
  });

  pi.on("agent_end", () => {
    post({ event: "agent_end" });
  });

  pi.on("tool_execution_start", (event: { toolCallId?: string; toolName?: string; args?: unknown }) => {
    post({
      event: "tool_start",
      call_id: event?.toolCallId,
      tool: event?.toolName,
      args: event?.args ?? {},
    });
  });

  pi.on("tool_execution_end", (event: { toolCallId?: string; toolName?: string; result?: unknown; isError?: boolean }) => {
    post({
      event: "tool_end",
      call_id: event?.toolCallId,
      tool: event?.toolName,
      result: event?.result ?? null,
      is_error: Boolean(event?.isError),
    });
  });
}
`;
}
