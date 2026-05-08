import { useState } from "react";

interface Props {
  functionId: string;
  args: unknown;
}

export function FunctionCallBlock({ functionId, args }: Props) {
  const [open, setOpen] = useState(false);
  return (
    <div className="block block-tool-use" data-open={open}>
      <button
        type="button"
        className="block-head"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="block-eyebrow">function</span>
        <span className="block-title">{functionId}</span>
        <span className="block-toggle">{open ? "−" : "+"}</span>
      </button>
      {open ? (
        <pre className="block-body">{JSON.stringify(args, null, 2)}</pre>
      ) : null}
    </div>
  );
}
