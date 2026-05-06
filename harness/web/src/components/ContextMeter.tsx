import type { AgentMessage, AssistantMessage } from "../types";

interface Props {
  messages: AgentMessage[];
  contextWindow: number | null;
  model: string;
}

const fmt = new Intl.NumberFormat("en-US");

function lastAssistant(messages: AgentMessage[]): AssistantMessage | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role === "assistant") return m;
  }
  return null;
}

export function ContextMeter({ messages, contextWindow, model }: Props) {
  const last = lastAssistant(messages);
  const used = last
    ? (last.usage?.input ?? 0) + (last.usage?.output ?? 0)
    : 0;
  const cap = contextWindow ?? 0;
  const ratio = cap > 0 ? Math.min(1, used / cap) : 0;
  const pct = cap > 0 ? ratio * 100 : 0;

  // tier the meter color: calm -> warn -> danger
  const tier = ratio < 0.8 ? "calm" : ratio < 0.95 ? "warn" : "danger";

  return (
    <div className="ctx-meter" data-tier={tier}>
      <div className="ctx-meter-row">
        <span className="ctx-meter-label">context</span>
        <span className="ctx-meter-numbers">
          <span className="ctx-meter-used">{fmt.format(used)}</span>
          <span className="ctx-meter-divider">/</span>
          <span className="ctx-meter-cap">
            {cap > 0 ? fmt.format(cap) : "—"}
          </span>
          <span className="ctx-meter-unit">tokens</span>
          {cap > 0 ? (
            <span className="ctx-meter-pct">
              {pct < 0.1 && pct > 0 ? "<0.1" : pct.toFixed(pct < 10 ? 2 : 1)}%
            </span>
          ) : null}
        </span>
        <span className="ctx-meter-model" title={model}>
          {model}
        </span>
      </div>
      <div className="ctx-meter-bar" aria-hidden>
        <div
          className="ctx-meter-fill"
          style={{ width: `${cap > 0 ? Math.max(pct, 0.5) : 0}%` }}
        />
      </div>
    </div>
  );
}
