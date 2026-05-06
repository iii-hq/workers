import { useEffect, useState } from "react";
import { bridge } from "../bridge";
import type { HarnessStatus } from "../types";

export function StatusPill() {
  const [status, setStatus] = useState<"unknown" | "ok" | "down">("unknown");
  const [info, setInfo] = useState<HarnessStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const s = await bridge<HarnessStatus>("harness::status");
        if (!cancelled) {
          setStatus("ok");
          setInfo(s);
        }
      } catch {
        if (!cancelled) setStatus("down");
      }
    };
    tick();
    const id = setInterval(tick, 5000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  return (
    <div className="status-pill" data-status={status}>
      <span className="status-dot" aria-hidden />
      <span className="status-label">
        {status === "ok" && info
          ? `${info.name} ${info.version} · ${info.expected_workers.length} workers`
          : status === "down"
            ? "engine unreachable"
            : "checking…"}
      </span>
    </div>
  );
}
