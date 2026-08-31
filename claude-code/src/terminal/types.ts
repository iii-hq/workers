/**
 * The terminal half speaks the same AgentEvent wire shapes as the headless
 * half — a terminal turn and a `claude::run` turn must render identically in
 * the console — so the types come from `../types.js` and only the Claude Code
 * hook payload is new here.
 */

export type {
  AgentEvent,
  AgentMessage,
  AssistantMessage,
  ContentBlock,
  FunctionCallContent,
  FunctionResultMessage,
  TextContent,
  UserMessage,
} from '../types.js';

/** The Claude Code hook payload, as the workspace hooks post it. */
export type HookEvent = {
  hook_event_name?: string;
  session_id?: string;
  cwd?: string;
  prompt?: string;
  tool_name?: string;
  tool_use_id?: string;
  tool_input?: Record<string, unknown>;
  tool_response?: unknown;
};
