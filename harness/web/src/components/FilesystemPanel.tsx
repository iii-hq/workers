import { useCallback, useEffect, useMemo, useState } from "react";
import { bridge, BridgeError } from "../bridge";
import type {
  FsEntry,
  FsLsResponse,
  FsReadDetails,
  ToolResult,
} from "../types";

function joinPath(base: string, name: string): string {
  if (base === "/" || base === "") return `/${name}`.replace(/^\/+/, "/");
  return `${base.replace(/\/$/, "")}/${name}`;
}

function parentPath(p: string): string {
  if (!p || p === "/") return "/";
  const trimmed = p.replace(/\/$/, "");
  const idx = trimmed.lastIndexOf("/");
  if (idx <= 0) return "/";
  return trimmed.slice(0, idx);
}

function fmtSize(n: number | undefined): string {
  if (n == null) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function entryKind(e: FsEntry): "dir" | "file" {
  if (e.is_dir === true) return "dir";
  if (e.kind === "dir") return "dir";
  if (typeof e.mode === "string" && e.mode.startsWith("d")) return "dir";
  return "file";
}

export function FilesystemPanel() {
  const [path, setPath] = useState<string>("/");
  const [pathDraft, setPathDraft] = useState<string>("/");
  const [entries, setEntries] = useState<FsEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [previewText, setPreviewText] = useState<string>("");
  const [previewMeta, setPreviewMeta] = useState<FsReadDetails | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);

  const listDir = useCallback(async (target: string) => {
    setLoading(true);
    setError(null);
    setEntries(null);
    try {
      const result = await bridge<FsLsResponse>(
        "shell::fs::ls",
        { path: target },
      );
      setEntries(result.entries ?? []);
      setPath(target);
      setPathDraft(target);
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, []);

  const readFile = useCallback(async (target: string) => {
    setPreviewLoading(true);
    setPreviewError(null);
    setPreviewPath(target);
    setPreviewText("");
    setPreviewMeta(null);
    try {
      const result = await bridge<ToolResult<FsReadDetails>>(
        "harness::fs::read_inline",
        { path: target },
      );
      if (result.details?.error) {
        throw new BridgeError(
          result.details.error,
          "harness::fs::read_inline",
          0,
        );
      }
      const text = result.content?.[0]?.text ?? "";
      setPreviewText(text);
      setPreviewMeta(result.details ?? null);
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setPreviewError(msg);
    } finally {
      setPreviewLoading(false);
    }
  }, []);

  useEffect(() => {
    void listDir("/");
  }, [listDir]);

  const sortedEntries = useMemo(() => {
    if (!entries) return [];
    return [...entries].sort((a, b) => {
      const ka = entryKind(a);
      const kb = entryKind(b);
      if (ka !== kb) return ka === "dir" ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }, [entries]);

  return (
    <section className="fs-panel">
      <header className="panel-head">
        <span className="panel-title">filesystem</span>
        <span className="panel-sub">sandbox via shell::fs::*</span>
      </header>

      <form
        className="fs-pathbar"
        onSubmit={(e) => {
          e.preventDefault();
          void listDir(pathDraft.trim() || "/");
        }}
      >
        <button
          type="button"
          className="btn-ghost"
          onClick={() => void listDir(parentPath(path))}
          disabled={loading || path === "/"}
          aria-label="parent directory"
        >
          ↑
        </button>
        <input
          className="fs-pathinput"
          value={pathDraft}
          onChange={(e) => setPathDraft(e.target.value)}
          spellCheck={false}
        />
        <button
          type="submit"
          className="btn-primary"
          disabled={loading}
        >
          {loading ? "loading…" : "open"}
        </button>
      </form>

      {error ? (
        <p className="panel-error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="fs-body">
        <ul className="fs-list">
          {sortedEntries.length === 0 && !loading && !error ? (
            <li className="fs-empty">empty directory</li>
          ) : null}
          {sortedEntries.map((e) => {
            const kind = entryKind(e);
            const full = joinPath(path, e.name);
            const isPreview = previewPath === full;
            return (
              <li key={full}>
                <button
                  type="button"
                  className="fs-row"
                  data-kind={kind}
                  data-active={isPreview}
                  onClick={() => {
                    if (kind === "dir") void listDir(full);
                    else void readFile(full);
                  }}
                >
                  <span className="fs-icon" aria-hidden>
                    {kind === "dir" ? "▸" : "·"}
                  </span>
                  <span className="fs-name">{e.name}</span>
                  <span className="fs-meta">
                    {kind === "file" ? fmtSize(e.size) : "dir"}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>

        <div className="fs-preview">
          {previewPath == null ? (
            <p className="fs-preview-empty">
              click a file to preview · max 256 KB inline
            </p>
          ) : (
            <>
              <header className="fs-preview-head">
                <span className="fs-preview-path">{previewPath}</span>
                {previewMeta?.truncated ? (
                  <span className="budget-tag" data-tone="warn">
                    truncated
                  </span>
                ) : null}
                {previewMeta?.bytes_read != null ? (
                  <span className="fs-preview-size">
                    {fmtSize(previewMeta.bytes_read)}
                  </span>
                ) : null}
              </header>
              {previewLoading ? (
                <p className="fs-preview-loading">loading…</p>
              ) : previewError ? (
                <p className="panel-error" role="alert">
                  {previewError}
                </p>
              ) : (
                <pre className="fs-preview-body">{previewText}</pre>
              )}
            </>
          )}
        </div>
      </div>
    </section>
  );
}
