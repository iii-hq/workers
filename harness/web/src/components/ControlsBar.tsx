import type { AuthStatus, ModelInfo } from "../types";

interface Props {
  providers: readonly string[];
  provider: string;
  onProvider: (p: string) => void;

  models: ModelInfo[];
  model: string;
  onModel: (id: string) => void;

  authStatus: AuthStatus | null;
  onManageAuth: () => void;
}

export function ControlsBar({
  providers,
  provider,
  onProvider,
  models,
  model,
  onModel,
  authStatus,
  onManageAuth,
}: Props) {
  const providerModels = models.filter((m) => m.provider === provider);
  const tone =
    authStatus?.configured === true
      ? "ok"
      : authStatus === null
        ? "unknown"
        : "missing";

  return (
    <div className="controls">
      <div className="controls-group">
        <span className="controls-label">provider</span>
        <div className="seg" role="radiogroup" aria-label="provider">
          {providers.map((p) => (
            <button
              key={p}
              role="radio"
              aria-checked={p === provider}
              type="button"
              className="seg-btn"
              data-active={p === provider}
              onClick={() => onProvider(p)}
            >
              {p}
            </button>
          ))}
        </div>
      </div>

      <div className="controls-group">
        <span className="controls-label">model</span>
        <select
          className="model-select"
          value={model}
          onChange={(e) => onModel(e.target.value)}
          disabled={providerModels.length === 0}
        >
          {providerModels.length === 0 ? (
            <option value="">no models for {provider}</option>
          ) : (
            providerModels.map((m) => (
              <option key={m.id} value={m.id}>
                {m.id}
              </option>
            ))
          )}
        </select>
      </div>

      <button
        type="button"
        className="cred-pill"
        data-tone={tone}
        onClick={onManageAuth}
        title={
          authStatus?.label
            ? `${authStatus.label} (${authStatus.source})`
            : "no credential set"
        }
      >
        <span className="cred-dot" aria-hidden />
        <span>
          {tone === "ok"
            ? `credential · ${authStatus!.source}`
            : tone === "unknown"
              ? "credential · checking…"
              : "credential · set key"}
        </span>
      </button>
    </div>
  );
}
