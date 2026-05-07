// Per-message kebab actions: fork-from-here and copy-text.
//
// Fork is disabled when entry_id is null — that signals state↔tree drift
// (the message came from the state::* fallback in loadMessagesWithEntryIds
// because session-tree didn't have it). The tooltip points the user at
// /repair to recover.

import type { AgentMessage } from "../types";

interface Props {
  entryId: string | null;
  message: AgentMessage;
  onFork: (entryId: string) => void | Promise<void>;
}

function copyText(m: AgentMessage): void {
  const text = Array.isArray(m.content)
    ? m.content
        .filter(
          (b): b is { type: "text"; text: string } =>
            (b as { type?: string }).type === "text",
        )
        .map((b) => b.text)
        .join("\n")
    : String(m.content);
  if (navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(text);
  }
}

export function MessageActions({ entryId, message, onFork }: Props) {
  const disabled = entryId === null;
  return (
    <div className="msg-actions">
      <button
        type="button"
        className="msg-action"
        disabled={disabled}
        title={
          disabled
            ? "session-tree drift — run /repair to enable fork"
            : "Fork from here"
        }
        onClick={() => {
          if (entryId !== null) void onFork(entryId);
        }}
      >
        ↳ fork
      </button>
      <button
        type="button"
        className="msg-action"
        onClick={() => copyText(message)}
        title="Copy message text"
      >
        ⧉ copy
      </button>
    </div>
  );
}
