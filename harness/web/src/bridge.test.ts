// Regression coverage for the "error · [object Object]" bug: the iii-sdk's
// trigger() rejects with the raw wire payload (a plain object), not an Error
// instance. The earlier String(e) fallback collapsed every engine error to
// "[object Object]" in the UI. toBridgeError now walks common engine-error
// shapes and falls back to JSON.stringify so the real message reaches the
// user.

import { describe, expect, it } from "vitest";

import { BridgeError, toBridgeError } from "./bridge";

describe("toBridgeError", () => {
  it("preserves Error.message when an Error instance is thrown", () => {
    const err = toBridgeError(new Error("boom"), "auth::set_token");
    expect(err).toBeInstanceOf(BridgeError);
    expect(err.message).toBe("boom");
    expect(err.functionId).toBe("auth::set_token");
  });

  it("extracts message field from engine error payload", () => {
    // The shape iii-sdk hands us when the engine fails an invocation.
    const wire = { message: "missing required field: provider", error_code: "BAD_INPUT" };
    const err = toBridgeError(wire, "auth::set_token");
    expect(err.message).toBe("missing required field: provider");
  });

  it("falls back to `error` field when `message` is absent", () => {
    const wire = { error: "store.set failed: state worker not available" };
    const err = toBridgeError(wire, "auth::set_token");
    expect(err.message).toBe("store.set failed: state worker not available");
  });

  it("captures error_id when present", () => {
    const wire = { message: "boom", error_id: "abc-123" };
    const err = toBridgeError(wire, "auth::set_token");
    expect(err.errorId).toBe("abc-123");
  });

  it("falls back to JSON.stringify rather than '[object Object]'", () => {
    // The regression case: an engine error without any known string field.
    // Before the fix this rendered as "[object Object]". After the fix the
    // user sees enough JSON to diagnose, even for unfamiliar shapes.
    const wire = { code: 42, detail: { nested: true } };
    const err = toBridgeError(wire, "auth::set_token");
    expect(err.message).not.toBe("[object Object]");
    expect(err.message).toContain("\"code\":42");
  });

  it("never produces empty messages for primitive throws", () => {
    expect(toBridgeError("string thrown", "f").message).toBe("string thrown");
    expect(toBridgeError(42, "f").message).toBe("42");
    expect(toBridgeError(null, "f").message).toBe("null");
    expect(toBridgeError(undefined, "f").message).toBe("undefined");
  });

  it("ignores empty-string fields (otherwise the chain would short-circuit)", () => {
    const wire = { message: "", error: "fallback works" };
    const err = toBridgeError(wire, "f");
    expect(err.message).toBe("fallback works");
  });
});
