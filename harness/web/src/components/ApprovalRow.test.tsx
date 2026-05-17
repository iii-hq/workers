// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("../bridge", () => ({
  bridge: vi.fn(),
  BridgeError: class BridgeError extends Error {},
}));

import { bridge } from "../bridge";
import { ApprovalRow } from "./ApprovalRow";
import type { PendingApproval } from "../types";

const bridgeMock = bridge as unknown as ReturnType<typeof vi.fn>;

function approval(overrides: Partial<PendingApproval> = {}): PendingApproval {
  return {
    function_call_id: "tc-1",
    function_id: "shell::exec",
    args: { command: "echo", args: ["ok"] },
    expires_at: Date.now() + 60_000,
    ...overrides,
  };
}

describe("ApprovalRow", () => {
  afterEach(() => {
    cleanup();
    bridgeMock.mockReset();
    vi.useRealTimers();
  });

  it("sends always=true when allow + always is clicked", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: true, cascaded: 1 });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);

    fireEvent.click(screen.getByRole("button", { name: "allow + always" }));

    await waitFor(() => expect(bridgeMock).toHaveBeenCalledTimes(1));
    expect(bridgeMock).toHaveBeenCalledWith("approval::resolve", {
      session_id: "sess-1",
      function_call_id: "tc-1",
      tool_call_id: "tc-1",
      decision: "allow",
      always: true,
    });
  });

  it("sends user_corrected denial when feedback is present", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: true });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);

    fireEvent.change(screen.getByPlaceholderText(/wrong directory/i), {
      target: { value: "use /tmp/safer instead" },
    });
    fireEvent.click(screen.getByRole("button", { name: "deny" }));

    await waitFor(() => expect(bridgeMock).toHaveBeenCalledTimes(1));
    expect(bridgeMock).toHaveBeenCalledWith("approval::resolve", {
      session_id: "sess-1",
      function_call_id: "tc-1",
      tool_call_id: "tc-1",
      decision: "deny",
      denial: {
        kind: "user_corrected",
        detail: { feedback: "use /tmp/safer instead" },
      },
    });
  });

  it("keeps legacy tool_call_id as the resolve id when function_call_id is absent", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: true });
    render(
      <ApprovalRow
        sessionId="sess-1"
        pending={[
          approval({
            function_call_id: undefined,
            tool_call_id: "legacy-tc-1",
            function_id: undefined,
            tool_name: "shell::fs::write",
          }),
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "allow" }));

    await waitFor(() => expect(bridgeMock).toHaveBeenCalledTimes(1));
    expect(bridgeMock).toHaveBeenCalledWith("approval::resolve", {
      session_id: "sess-1",
      function_call_id: "legacy-tc-1",
      tool_call_id: "legacy-tc-1",
      decision: "allow",
    });
  });

  it("disables every action when the approval is expired", () => {
    vi.useFakeTimers();
    vi.setSystemTime(100_000);
    render(
      <ApprovalRow
        sessionId="sess-1"
        pending={[approval({ expires_at: 99_000 })]}
      />,
    );

    expect((screen.getByRole("button", { name: "deny" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "allow + always" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "allow" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("expired")).toBeTruthy();
    expect(bridgeMock).not.toHaveBeenCalled();
  });
});
