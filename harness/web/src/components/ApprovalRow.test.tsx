// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApprovalRow } from "./ApprovalRow";
import { bridge } from "../bridge";

vi.mock("../bridge", () => {
  class BridgeError extends Error {
    constructor(message: string) {
      super(message);
      this.name = "BridgeError";
    }
  }
  return {
    BridgeError,
    bridge: vi.fn(),
  };
});

const mockedBridge = vi.mocked(bridge);

describe("ApprovalRow", () => {
  beforeEach(() => {
    mockedBridge.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("surfaces approval resolve ok:false responses", async () => {
    mockedBridge.mockResolvedValueOnce({
      ok: false,
      error: "already_resolved",
    });

    render(
      <ApprovalRow
        sessionId="sess-a"
        pending={[
          {
            function_call_id: "tc-1",
            function_id: "shell::exec",
            args: { command: "date" },
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "allow" }));

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("already_resolved");
    });
  });

  it("renders wake failures even when no approval is pending", () => {
    render(
      <ApprovalRow
        sessionId="sess-a"
        pending={[]}
        wakeFailures={[{ error: "run::resume timed out", ts: Date.now() }]}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("run::resume timed out");
  });
});
