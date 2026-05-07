import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { AgentMessage } from "./types";

vi.mock("./bridge", () => ({
  bridge: vi.fn(),
  BridgeError: class BridgeError extends Error {},
}));

import { bridge } from "./bridge";
import { exportJson, exportMd } from "./export";

const bridgeMock = bridge as unknown as ReturnType<typeof vi.fn>;

interface CapturedDownload {
  filename: string;
  content: string;
  mime: string;
}

let captured: CapturedDownload[] = [];
let originalURL: typeof globalThis.URL;
let originalBlob: typeof globalThis.Blob;
let originalDocument: typeof globalThis.document;

function installDomShims(): void {
  captured = [];

  // Capture Blob construction so we can read what was written.
  const blobs = new WeakMap<object, CapturedDownload>();

  // @ts-expect-error overriding for the test
  globalThis.Blob = class FakeBlob {
    constructor(parts: BlobPart[], opts?: { type?: string }) {
      const text = parts
        .map((p) => (typeof p === "string" ? p : String(p)))
        .join("");
      blobs.set(this, { filename: "", content: text, mime: opts?.type ?? "" });
    }
  };

  globalThis.URL = {
    createObjectURL: (b: object): string => {
      const meta = blobs.get(b);
      // Stash the meta on a query string so the click handler can correlate.
      const id = String(captured.length);
      // Pre-register with empty filename — the anchor stub patches it.
      captured.push({ ...(meta ?? { filename: "", content: "", mime: "" }) });
      return `blob:fake/${id}`;
    },
    revokeObjectURL: () => {},
  } as unknown as typeof URL;

  // Minimal document stub: createElement('a') yields an object that records
  // its `download` filename and `click()`s into the most-recent capture row.
  const elements: Array<{
    href: string;
    download: string;
    click: () => void;
    remove: () => void;
  }> = [];

  globalThis.document = {
    createElement: (tag: string) => {
      if (tag !== "a") throw new Error(`unexpected createElement(${tag})`);
      const el = {
        href: "",
        download: "",
        click() {
          // Find the matching captured row by URL index.
          const match = /^blob:fake\/(\d+)$/.exec(this.href);
          if (!match) return;
          const idx = Number(match[1]);
          if (captured[idx]) captured[idx].filename = this.download;
        },
        remove() {},
      };
      elements.push(el);
      return el;
    },
    body: { appendChild: () => {} },
  } as unknown as Document;
}

function restoreDomShims(): void {
  globalThis.URL = originalURL;
  globalThis.Blob = originalBlob;
  globalThis.document = originalDocument;
}

const userMsg = (text: string): AgentMessage => ({
  role: "user",
  content: [{ type: "text", text }],
  timestamp: 1,
});

const assistantMsg = (text: string): AgentMessage => ({
  role: "assistant",
  content: [{ type: "text", text }],
  timestamp: 2,
  model: "claude-opus-4-7",
});

describe("export", () => {
  beforeEach(() => {
    bridgeMock.mockReset();
    originalURL = globalThis.URL;
    originalBlob = globalThis.Blob;
    originalDocument = globalThis.document;
    installDomShims();
  });

  afterEach(() => {
    restoreDomShims();
  });

  describe("exportMd", () => {
    it("calls session-tree::messages and writes a markdown blob", async () => {
      bridgeMock.mockResolvedValueOnce({
        messages: [
          { entry_id: "e1", message: userMsg("hello") },
          { entry_id: "e2", message: assistantMsg("world") },
        ],
      });
      await exportMd("s1");
      expect(bridgeMock).toHaveBeenCalledWith("session-tree::messages", {
        session_id: "s1",
      });
      expect(captured.length).toBe(1);
      expect(captured[0].filename).toBe("s1.md");
      expect(captured[0].mime).toBe("text/markdown");
      expect(captured[0].content).toContain("# Session s1");
      expect(captured[0].content).toContain("## User");
      expect(captured[0].content).toContain("hello");
      expect(captured[0].content).toContain("## Assistant");
      expect(captured[0].content).toContain("world");
    });

    it("renders tool_result rows as fenced JSON", async () => {
      const toolResult: AgentMessage = {
        role: "tool_result",
        content: [{ type: "text", text: "ok" }],
        tool_call_id: "tc1",
        tool_name: "shell::ls",
        is_error: false,
        timestamp: 3,
      };
      bridgeMock.mockResolvedValueOnce({
        messages: [{ entry_id: "e1", message: toolResult }],
      });
      await exportMd("s2");
      expect(captured[0].content).toContain("## Tool result (tc1)");
      expect(captured[0].content).toContain("```json");
    });
  });

  describe("exportJson", () => {
    it("wraps messages and tree under a metadata envelope", async () => {
      bridgeMock.mockResolvedValueOnce({
        messages: [{ entry_id: "e1", message: userMsg("hi") }],
      });
      bridgeMock.mockResolvedValueOnce({ root: { children: [] } });
      await exportJson("s3");
      expect(bridgeMock).toHaveBeenNthCalledWith(1, "session-tree::messages", {
        session_id: "s3",
      });
      expect(bridgeMock).toHaveBeenNthCalledWith(2, "session-tree::tree", {
        session_id: "s3",
      });
      expect(captured.length).toBe(1);
      expect(captured[0].filename).toBe("s3.json");
      expect(captured[0].mime).toBe("application/json");
      const parsed = JSON.parse(captured[0].content) as {
        session_id: string;
        exported_at: string;
        messages: unknown[];
        tree: unknown;
      };
      expect(parsed.session_id).toBe("s3");
      expect(parsed.messages.length).toBe(1);
      expect(parsed.tree).toEqual({ root: { children: [] } });
      expect(typeof parsed.exported_at).toBe("string");
    });

    it("tolerates a tree fetch failure (drift case) — tree becomes null", async () => {
      bridgeMock.mockResolvedValueOnce({
        messages: [{ entry_id: "e1", message: userMsg("x") }],
      });
      bridgeMock.mockRejectedValueOnce(new Error("no tree"));
      await exportJson("s4");
      const parsed = JSON.parse(captured[0].content);
      expect(parsed.tree).toBeNull();
    });
  });
});
