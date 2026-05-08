import { useEffect, useRef } from "react";
import type { UIEvent } from "react";
import type { AgentMessage } from "../types";
import { MessageActions } from "./MessageActions";
import { FunctionCallBlock } from "./FunctionCallBlock";
import { FunctionResultBlock } from "./FunctionResultBlock";
import { Markdown } from "./Markdown";

const STICK_THRESHOLD_PX = 64;

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

function shortModel(model: string): string {
  // claude-opus-4-7 → opus 4.7, claude-3-5-sonnet → sonnet 3.5
  const claudeNamed = model.match(/^claude-([a-z]+)-(\d+)-(\d+)$/i);
  if (claudeNamed) return `${claudeNamed[1]} ${claudeNamed[2]}.${claudeNamed[3]}`.toLowerCase();
  const claudeDated = model.match(/^claude-(\d+)-(\d+)-([a-z]+)/i);
  if (claudeDated) return `${claudeDated[3]} ${claudeDated[1]}.${claudeDated[2]}`.toLowerCase();
  const gpt = model.match(/^gpt-(.+)$/i);
  if (gpt) return `gpt-${gpt[1].replace(/-/g, " ")}`.toLowerCase();
  return model;
}

function roleLabel(m: AgentMessage): string {
  if (m.role === "user") return "you";
  if (m.role === "assistant") return m.model ? shortModel(m.model) : "the agent";
  if (m.role === "tool_result" || m.role === "function_result") return "function";
  return "message";
}

function blockText(b: any): string {
  if (typeof b === "string") return b;
  if (b && typeof b === "object") {
    if (typeof b.text === "string") return b.text;
    if (typeof b.content === "string") return b.content;
    if (Array.isArray(b.content)) return b.content.map(blockText).join("\n");
  }
  return JSON.stringify(b);
}

function renderBlocks(m: AgentMessage) {
  // Function-result rows: render the whole message as a single collapsible
  // result block instead of a wall of body text.
  if (m.role === "tool_result" || m.role === "function_result") {
    const fid =
      (m as { function_id?: string }).function_id ??
      (m as { tool_name?: string }).tool_name ??
      "function";
    const output = m.content.map(blockText).join("\n");
    return (
      <FunctionResultBlock
        key="result"
        functionId={fid}
        isError={Boolean((m as { is_error?: boolean }).is_error)}
        output={output}
      />
    );
  }

  return m.content.map((b: any, i: number) => {
    if (b.type === "text") {
      if (m.role === "assistant") {
        return (
          <div key={i} className="msg-text">
            <Markdown text={b.text} />
          </div>
        );
      }
      return <p key={i} className="msg-text">{b.text}</p>;
    }
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
      const text = blockText(b);
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
  const viewRef = useRef<HTMLElement | null>(null);
  const stickRef = useRef(true);

  useEffect(() => {
    stickRef.current = true;
    const el = viewRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [sessionId]);

  useEffect(() => {
    if (!stickRef.current) return;
    const el = viewRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, loading]);

  function onScroll(e: UIEvent<HTMLElement>) {
    const el = e.currentTarget;
    const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
    stickRef.current = dist < STICK_THRESHOLD_PX;
  }

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
    <section className="view" ref={viewRef} onScroll={onScroll}>
      <header className="view-head">
        <span className="view-eyebrow">session</span>
        <h2 className="view-title">{sessionId}</h2>
      </header>
      <ol className="messages">
        {messages.map((m, i) => {
          const prevRole = i > 0 ? messages[i - 1].role : null;
          const isTurnChange = prevRole !== null && prevRole !== m.role;
          return (
          <li
            key={i}
            className={`msg${isTurnChange ? " msg-turn-change" : ""}`}
            data-role={m.role}
          >
            <span className="msg-role">{roleLabel(m)}</span>
            <div className="msg-body">
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
            </div>
          </li>
          );
        })}
        {loading ? (
          <li className="msg msg-loading">
            <span className="msg-role">…</span>
            <div className="msg-body">
              <p className="msg-text">running turn…</p>
            </div>
          </li>
        ) : null}
      </ol>
    </section>
  );
}
