import type { AgentMessage } from "../types";

interface Props {
  sessionId: string;
  messages: AgentMessage[];
  loading: boolean;
}

function blockText(m: AgentMessage): string {
  return m.content
    .filter((b): b is { type: "text"; text: string } => b.type === "text")
    .map((b) => b.text)
    .join("\n");
}

function roleLabel(m: AgentMessage): string {
  if (m.role === "user") return "you";
  if (m.role === "assistant") return m.model ? `${m.model}` : "assistant";
  return m.tool_name ? `tool · ${m.tool_name}` : "tool";
}

export function SessionView({ sessionId, messages, loading }: Props) {
  if (!sessionId) {
    return (
      <section className="view view-empty">
        <h2 className="view-empty-h">no session selected</h2>
        <p className="view-empty-p">
          start a new turn or pick one from the rail.
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
        {messages.map((m, i) => {
          const text = blockText(m);
          if (!text) return null;
          return (
            <li key={i} className="msg" data-role={m.role}>
              <span className="msg-role">{roleLabel(m)}</span>
              <p className="msg-text">{text}</p>
              {m.role === "assistant" && m.usage ? (
                <p className="msg-usage">
                  {m.usage.input ?? 0}↓ · {m.usage.output ?? 0}↑ tokens
                  {m.stop_reason ? ` · stop: ${m.stop_reason}` : null}
                </p>
              ) : null}
            </li>
          );
        })}
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
