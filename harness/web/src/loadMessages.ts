import { bridge } from "./bridge";
import type { AgentMessage } from "./types";

export interface MessageWithEntryId {
  entry_id: string | null;
  message: AgentMessage;
}

interface SessionTreeMessagesResponse {
  messages: { entry_id: string; message: AgentMessage }[];
}

/**
 * Load a session's messages with entry_ids from session-tree, falling back
 * to state::get on empty/error. entry_id is `null` for fallback rows;
 * fork features must treat null as "fork disabled — run /repair".
 */
export async function loadMessagesWithEntryIds(
  sessionId: string,
): Promise<MessageWithEntryId[]> {
  try {
    const tree = await bridge<SessionTreeMessagesResponse>("session-tree::messages", {
      session_id: sessionId,
    });
    if (tree?.messages?.length) {
      return tree.messages.map((p) => ({ entry_id: p.entry_id, message: p.message }));
    }
  } catch {
    // fall through to state::* fallback
  }

  try {
    const bare = await bridge<AgentMessage[] | null>("state::get", {
      scope: "agent",
      key: `session/${sessionId}/messages`,
    });
    if (Array.isArray(bare)) {
      return bare.map((message) => ({ entry_id: null, message }));
    }
  } catch {
    // both stores unreachable
  }

  return [];
}
