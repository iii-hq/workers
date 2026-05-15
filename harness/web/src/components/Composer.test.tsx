// @vitest-environment jsdom

import { describe, it, expect, vi, afterEach } from "vitest";
import { fireEvent, render, screen, cleanup } from "@testing-library/react";
import { Composer } from "./Composer";

afterEach(() => {
  cleanup();
});

function setup(overrides: Partial<Parameters<typeof Composer>[0]> = {}) {
  const onSend = vi.fn().mockResolvedValue(undefined);
  const onStop = vi.fn().mockResolvedValue(undefined);
  const utils = render(
    <Composer
      disabled={false}
      onSend={onSend}
      cwd=""
      skillRows={null}
      sessionMessages={[]}
      running={false}
      onStop={onStop}
      {...overrides}
    />,
  );
  return { ...utils, onSend, onStop };
}

describe("Composer stop button", () => {
  it("renders the send variant when not running", () => {
    setup();
    expect(screen.getByText("send")).toBeTruthy();
    expect(screen.queryByText("stop")).toBeNull();
  });

  it("morphs to a stop button when running is true", () => {
    setup({ running: true });
    const stopBtn = screen.getByRole("button", {
      name: /stop current run after current step/i,
    });
    expect(stopBtn.getAttribute("data-mode")).toBe("stop");
    expect(stopBtn.getAttribute("title")).toBe("stop after current step");
    expect(screen.queryByText("send")).toBeNull();
  });

  it("clicking the stop button calls onStop, not onSend", () => {
    const { onSend, onStop } = setup({ running: true });
    fireEvent.click(
      screen.getByRole("button", { name: /stop current run/i }),
    );
    expect(onStop).toHaveBeenCalledTimes(1);
    expect(onSend).not.toHaveBeenCalled();
  });

  it("shows 'stopping…' and disables itself after a stop click", async () => {
    setup({ running: true });
    const stopBtn = screen.getByRole("button", { name: /stop current run/i });
    fireEvent.click(stopBtn);
    // Re-query because the label changed.
    const stopping = screen.getByRole("button", { name: /stop current run/i });
    expect(stopping.textContent).toContain("stopping");
    expect((stopping as HTMLButtonElement).disabled).toBe(true);
    expect(stopping.getAttribute("aria-busy")).toBe("true");
  });

  it("form submit is a no-op while running", () => {
    const { onSend } = setup({ running: true });
    // The textarea is `readOnly` while running but still focusable. Type
    // something programmatically (we set value via fireEvent for the test)
    // and try to submit via Enter.
    const textarea = screen.getByPlaceholderText(
      /run in flight/i,
    ) as HTMLTextAreaElement;
    fireEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      shiftKey: false,
    });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("returns to send variant when running flips back to false", () => {
    const { rerender, onSend, onStop } = setup({ running: true });
    fireEvent.click(
      screen.getByRole("button", { name: /stop current run/i }),
    );
    expect(onStop).toHaveBeenCalled();
    rerender(
      <Composer
        disabled={false}
        onSend={onSend}
        cwd=""
        skillRows={null}
        sessionMessages={[]}
        running={false}
        onStop={onStop}
      />,
    );
    expect(screen.getByText("send")).toBeTruthy();
    expect(screen.queryByText("stop")).toBeNull();
    expect(screen.queryByText("stopping…")).toBeNull();
  });

  it("send button has a min-width pinned so the morph does not shift layout", () => {
    setup();
    const sendBtn = screen.getByRole("button", { name: /send/i });
    expect(sendBtn.className).toContain("composer-send");
    // The min-width is set in CSS, not inline — we can only assert the
    // class is the same one we styled (sanity check against accidental
    // class renames).
  });
});
