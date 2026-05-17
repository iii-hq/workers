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

  // ──────────────────────────────────────────────────────────────────
  // Finding #8: { ok: false } replies from approval::resolve must
  // surface as operator-visible alerts (not silently dismissed). Each
  // typed error code is mapped to distinct copy so the operator can
  // tell a multi-window race (`already_resolved`) from a stale
  // approval (`timed_out`) from a now-executing call (`in_flight`).
  // ──────────────────────────────────────────────────────────────────

  it("ok:true clears prior state and emits no alert", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: true });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);
    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    await waitFor(() => expect(bridgeMock).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("ok:false / timed_out surfaces an operator-readable alert", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: false, error: "timed_out" });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);
    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/expired before you could resolve it/i);
  });

  it("ok:false / already_resolved is distinct from timed_out (multi-window race signal)", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: false, error: "already_resolved" });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);
    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/another window already resolved/i);
    expect(alert.textContent).not.toMatch(/expired/i);
  });

  it("ok:false / in_flight surfaces a specific mid-execution message", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: false, error: "in_flight" });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);
    fireEvent.click(screen.getByRole("button", { name: "deny" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/mid-execution/i);
  });

  it("ok:false / not_found surfaces with a record-missing message", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: false, error: "not_found" });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);
    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/not found/i);
  });

  it("unknown error code falls through to a generic resolve-failed line", async () => {
    bridgeMock.mockResolvedValueOnce({ ok: false, error: "weird_new_code" });
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);
    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/resolve failed: weird_new_code/i);
  });

  it("transport errors (thrown) still surface via the catch path", async () => {
    bridgeMock.mockRejectedValueOnce(new Error("ws closed"));
    render(<ApprovalRow sessionId="sess-1" pending={[approval()]} />);
    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/ws closed/i);
  });
});
