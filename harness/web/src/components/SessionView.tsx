import type { AgentMessage } from "../types";
import { ToolUseBlock } from "./ToolUseBlock";
import { ToolResultBlock } from "./ToolResultBlock";

interface Props {
  sessionId: string;
  messages: AgentMessage[];
  loading: boolean;
}

function roleLabel(m: AgentMessage): string {
  if (m.role === "user") return "you";
  if (m.role === "assistant") return m.model ? `${m.model}` : "assistant";
  return "tool";
}

function renderBlocks(m: AgentMessage) {
  return m.content.map((b: any, i: number) => {
    if (b.type === "text") return <p key={i} className="msg-text">{b.text}</p>;
    if (b.type === "tool_use" || b.type === "tool_call") {
      const args = b.input ?? b.arguments ?? {};
      return <ToolUseBlock key={i} name={b.name} args={args} />;
    }
    if (b.type === "tool_result") {
      const text = Array.isArray(b.content)
        ? b.content.map((c: any) => (typeof c === "string" ? c : c.text ?? JSON.stringify(c))).join("\n")
        : typeof b.content === "string"
          ? b.content
          : JSON.stringify(b.content);
      return (
        <ToolResultBlock
          key={i}
          toolName={b.tool_name ?? "tool"}
          isError={Boolean(b.is_error)}
          output={text}
        />
      );
    }
    return null;
  });
}

export function SessionView({ sessionId, messages, loading }: Props) {
  if (!sessionId) {
    return (
      <section className="view view-empty">
        <h2 className="view-empty-h">no session selected</h2>
        <p className="view-empty-p">start a new turn or pick one from the rail.</p>
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
