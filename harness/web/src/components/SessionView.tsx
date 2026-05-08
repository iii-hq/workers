import type { AgentMessage } from "../types";
import { MessageActions } from "./MessageActions";
import { FunctionCallBlock } from "./FunctionCallBlock";
import { FunctionResultBlock } from "./FunctionResultBlock";

interface Props {
  sessionId: string;
  messages: AgentMessage[];
  /**
   * Parallel array to `messages`. Each slot is the entry_id keying that
   * message in session-tree, or `null` when the message came from the
   * state::* fallback (drift case — fork is disabled for null entries).
   */
  messageEntryIds: (string | null)[];
  loading: boolean;
  onForkFromMessage: (entryId: string) => void | Promise<void>;
}

function roleLabel(m: AgentMessage): string {
  if (m.role === "user") return "you";
  if (m.role === "assistant") return m.model ? `${m.model}` : "assistant";
  if (m.role === "tool_result" || m.role === "function_result") return "function";
  return "message";
}

function renderBlocks(m: AgentMessage) {
  return m.content.map((b: any, i: number) => {
    if (b.type === "text") return <p key={i} className="msg-text">{b.text}</p>;
    if (
      b.type === "tool_use" ||
      b.type === "tool_call" ||
      b.type === "functionCall" ||
      b.type === "function_call"
    ) {
      const args = b.input ?? b.arguments ?? {};
      const fid =
        typeof b.function_id === "string"
          ? b.function_id
          : typeof b.name === "string"
            ? b.name
            : "unknown";
      return <FunctionCallBlock key={i} functionId={fid} args={args} />;
    }
    if (b.type === "tool_result" || b.type === "function_result" || b.type === "functionResult") {
      const text = Array.isArray(b.content)
        ? b.content.map((c: any) => (typeof c === "string" ? c : c.text ?? JSON.stringify(c))).join("\n")
        : typeof b.content === "string"
          ? b.content
          : JSON.stringify(b.content);
      const fid =
        typeof b.function_id === "string"
          ? b.function_id
          : typeof b.tool_name === "string"
            ? b.tool_name
            : "function";
      return (
        <FunctionResultBlock
          key={i}
          functionId={fid}
          isError={Boolean(b.is_error)}
          output={text}
        />
      );
    }
    return null;
  });
}

export function SessionView({
  sessionId,
  messages,
  messageEntryIds,
  loading,
  onForkFromMessage,
}: Props) {
  if (!sessionId) {
    return (
      <section className="view view-empty">
        <span className="view-empty-eyebrow">{"// no transcript"}</span>
        <h2 className="view-empty-h">A blank page.</h2>
        <p className="view-empty-p">
          Open a session from the sidebar, or address the agent below.
        </p>
        <p className="view-empty-foot">
          You're a solo operator on a local bus. The agent is listening.
        </p>
      </section>
    );
  }
  return (
    <section className="view">
      <header className="view-head">
        <span className="view-eyebrow">session</span>
        <h2 className="view-title">{sessionId}</h2>
      </header>
      <ol className="messages">
        {messages.map((m, i) => (
          <li key={i} className="msg" data-role={m.role}>
            <span className="msg-role">{roleLabel(m)}</span>
            {renderBlocks(m)}
            {m.role === "assistant" && m.usage ? (
              <p className="msg-usage">
                {m.usage.input ?? 0}↓ · {m.usage.output ?? 0}↑ tokens
                {m.stop_reason ? ` · stop: ${m.stop_reason}` : null}
              </p>
            ) : null}
            <MessageActions
              entryId={messageEntryIds[i] ?? null}
              message={m}
              onFork={onForkFromMessage}
            />
          </li>
        ))}
        {loading ? (
          <li className="msg msg-loading">
            <span className="msg-role">…</span>
            <p className="msg-text">running turn…</p>
          </li>
        ) : null}
      </ol>
    </section>
  );
}
