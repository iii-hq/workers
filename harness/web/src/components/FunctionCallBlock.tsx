import { useState } from "react";

interface Props {
  functionId: string;
  args: unknown;
}

function unwrapAgentCall(
  functionId: string,
  args: unknown,
): { eyebrow: string; title: string } {
  if (
    functionId === "agent_call" &&
    args &&
    typeof args === "object" &&
    typeof (args as { function?: unknown }).function === "string"
  ) {
    return {
      eyebrow: "function",
      title: (args as { function: string }).function,
    };
  }
  return { eyebrow: "function", title: functionId };
}

export function FunctionCallBlock({ functionId, args }: Props) {
  const [open, setOpen] = useState(false);
  const { eyebrow, title } = unwrapAgentCall(functionId, args);
  return (
    <div className="block block-tool-use" data-open={open}>
      <button
        type="button"
        className="block-head"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="block-eyebrow">{eyebrow}</span>
        <span className="block-title">{title}</span>
        <span className="block-toggle" aria-hidden="true">
          {open ? "−" : "+"}
        </span>
      </button>
      {open ? (
        <pre className="block-body">{JSON.stringify(args, null, 2)}</pre>
      ) : null}
    </div>
  );
}
