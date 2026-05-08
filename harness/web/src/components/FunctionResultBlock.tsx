import { useMemo, useState } from "react";
import { Markdown } from "./Markdown";

interface Props {
  functionId: string;
  isError: boolean;
  output: string;
}

const COLLAPSED_LIMIT = 4000;

type Format = "json" | "markdown" | "text";

function detectFormat(s: string): { format: Format; pretty: string } {
  const trimmed = s.trim();
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    try {
      const parsed = JSON.parse(trimmed);
      if (parsed !== null && typeof parsed === "object") {
        return { format: "json", pretty: JSON.stringify(parsed, null, 2) };
      }
    } catch {
      // fall through
    }
  }
  if (
    /^#{1,6}\s/m.test(trimmed) ||
    /^[-*+]\s/m.test(trimmed) ||
    /^\d+\.\s/m.test(trimmed) ||
    /```/.test(trimmed) ||
    /\[[^\]]+\]\([^)]+\)/.test(trimmed)
  ) {
    return { format: "markdown", pretty: s };
  }
  return { format: "text", pretty: s };
}

function previewLine(s: string, format: Format): string {
  if (format === "json") {
    return s.replace(/\s+/g, " ").trim();
  }
  const lines = s.trim().split("\n");
  const first = lines.find((l) => l.trim().length > 0) ?? "";
  if (format === "markdown") {
    return first.replace(/^#{1,6}\s+/, "").replace(/^[-*+]\s+/, "");
  }
  return first;
}

export function FunctionResultBlock({ functionId, isError, output }: Props) {
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const { format, pretty } = useMemo(() => detectFormat(output), [output]);
  const truncated = pretty.length > COLLAPSED_LIMIT;
  const visible =
    expanded || !truncated ? pretty : pretty.slice(0, COLLAPSED_LIMIT) + "…";
  const preview = previewLine(output, format);

  return (
    <div
      className="block block-tool-result"
      data-error={isError}
      data-open={open}
      data-format={format}
    >
      <button
        type="button"
        className="block-head"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="block-eyebrow">{isError ? "error" : "result"}</span>
        <span className="block-title">{functionId}</span>
        {!open && preview ? (
          <span className="block-preview">{preview}</span>
        ) : null}
        <span className="block-toggle" aria-hidden="true">
          {open ? "−" : "+"}
        </span>
      </button>
      {open ? (
        <>
          {format === "markdown" && !truncated ? (
            <div className="block-body block-body-md">
              <Markdown text={pretty} />
            </div>
          ) : (
            <pre className="block-body">{visible}</pre>
          )}
          {truncated ? (
            <button
              type="button"
              className="block-more"
              onClick={() => setExpanded((v) => !v)}
            >
              {expanded ? "show less" : "show more"}
            </button>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
