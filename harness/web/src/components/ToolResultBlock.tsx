import { useState } from "react";

interface Props {
  toolName: string;
  isError: boolean;
  output: string;
}

const COLLAPSED_LIMIT = 600;

export function ToolResultBlock({ toolName, isError, output }: Props) {
  const [expanded, setExpanded] = useState(false);
  const truncated = output.length > COLLAPSED_LIMIT;
  const visible = expanded || !truncated ? output : output.slice(0, COLLAPSED_LIMIT) + "…";
  return (
    <div className="block block-tool-result" data-error={isError}>
      <header className="block-head">
        <span className="block-eyebrow">{isError ? "error" : "result"}</span>
        <span className="block-title">{toolName}</span>
      </header>
      <pre className="block-body">{visible}</pre>
      {truncated ? (
        <button
          type="button"
          className="block-more"
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? "show less" : "show more"}
        </button>
      ) : null}
    </div>
  );
}
