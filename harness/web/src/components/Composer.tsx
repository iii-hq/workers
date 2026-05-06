import { FormEvent, useState } from "react";

interface Props {
  disabled: boolean;
  onSend: (prompt: string) => Promise<void>;
}

export function Composer({ disabled, onSend }: Props) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const trimmed = text.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    try {
      await onSend(trimmed);
      setText("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="composer" onSubmit={submit}>
      <textarea
        className="composer-input"
        placeholder={
          disabled
            ? "set a credential to begin"
            : "address the agent. shift+enter inserts a new line."
        }
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void submit(e);
          }
        }}
        disabled={disabled || busy}
        rows={3}
      />
      <button
        type="submit"
        className="composer-send"
        disabled={disabled || busy || !text.trim()}
      >
        <span className="composer-send-mark" aria-hidden>
          ⏎
        </span>
        <span className="composer-send-label">{busy ? "running" : "send"}</span>
      </button>
    </form>
  );
}
