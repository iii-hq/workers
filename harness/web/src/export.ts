// Client-side session exporters. Both call session-tree functions over the
// bridge and stream the result through a Blob → temporary <a download> click.
// The browser owns the file dialog; we never round-trip bytes to the server.
//
// Markdown: human-readable transcript with role headings; tool results are
// fenced JSON since their content shape is open-ended.
// JSON: structured wrapper { session_id, exported_at, messages, tree } —
// tree fetch is best-effort (drift case returns null).

import { bridge } from "./bridge";
import type { AgentMessage } from "./types";

interface MessagePair {
  entry_id: string;
  message: AgentMessage;
}

interface MessagesResponse {
  messages: MessagePair[];
}

function downloadBlob(filename: string, content: string, mime: string): void {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function formatContent(content: unknown): string {
  if (Array.isArray(content)) {
    return content
      .map((b) => {
        const block = b as { type?: string; text?: string };
        if (block?.type === "text") return `${block.text ?? ""}\n`;
        if (block?.type === "tool_use") {
          return `\`\`\`json tool_use\n${JSON.stringify(b, null, 2)}\n\`\`\`\n`;
        }
        return `\`\`\`json\n${JSON.stringify(b, null, 2)}\n\`\`\`\n`;
      })
      .join("\n");
  }
  return String(content);
}

export async function exportMd(sessionId: string): Promise<void> {
  const { messages } = await bridge<MessagesResponse>("session-tree::messages", {
    session_id: sessionId,
  });
  const lines: string[] = [`# Session ${sessionId}\n`];
  for (const { message } of messages) {
    if (message.role === "user") {
      lines.push("## User\n");
      lines.push(formatContent(message.content));
    } else if (message.role === "assistant") {
      lines.push("## Assistant\n");
      lines.push(formatContent(message.content));
    } else {
      // tool_result and any other roles fall through here. The JSON dump
      // preserves the full envelope (tool_call_id, is_error, content blocks).
      const toolId =
        (message as { tool_call_id?: string }).tool_call_id ?? "unknown";
      lines.push(`## Tool result (${toolId})\n`);
      lines.push(`\`\`\`json\n${JSON.stringify(message, null, 2)}\n\`\`\`\n`);
    }
  }
  downloadBlob(`${sessionId}.md`, lines.join("\n"), "text/markdown");
}

export async function exportJson(sessionId: string): Promise<void> {
  const [messagesResult, tree] = await Promise.all([
    bridge<MessagesResponse>("session-tree::messages", { session_id: sessionId }),
    bridge<unknown>("session-tree::tree", { session_id: sessionId }).catch(
      () => null,
    ),
  ]);
  const wrapper = {
    session_id: sessionId,
    exported_at: new Date().toISOString(),
    messages: messagesResult.messages,
    tree,
  };
  downloadBlob(
    `${sessionId}.json`,
    JSON.stringify(wrapper, null, 2),
    "application/json",
  );
}
