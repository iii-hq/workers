import { useCallback, useEffect, useState } from "react";
import { bridge, BridgeError } from "../bridge";
import type {
  Budget,
  BudgetForecast,
  BudgetUsage,
} from "../types";

interface BudgetDetail {
  budget: Budget;
  usage: BudgetUsage | null;
  forecast: BudgetForecast | null;
  error: string | null;
}

function fmtUsd(n: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 2,
  }).format(n);
}

function fmtPct(spent: number, ceiling: number): string {
  if (ceiling <= 0) return "—";
  return `${((spent / ceiling) * 100).toFixed(1)}%`;
}

function fmtDate(ms: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

async function loadDetail(b: Budget): Promise<BudgetDetail> {
  const out: BudgetDetail = {
    budget: b,
    usage: null,
    forecast: null,
    error: null,
  };
  try {
    out.usage = await bridge<BudgetUsage>("budget::usage", {
      budget_id: b.id,
      window: "all",
    });
  } catch (e) {
    out.error = e instanceof BridgeError ? e.message : String(e);
  }
  try {
    out.forecast = await bridge<BudgetForecast>("budget::forecast", {
      budget_id: b.id,
    });
  } catch (e) {
    if (!out.error) {
      out.error = e instanceof BridgeError ? e.message : String(e);
    }
  }
  return out;
}

export function CostPanel() {
  const [details, setDetails] = useState<BudgetDetail[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await bridge<{ budgets: Budget[] } | Budget[]>(
        "budget::list",
        {},
      );
      const list = Array.isArray(resp)
        ? resp
        : Array.isArray(resp?.budgets)
          ? resp.budgets
          : [];
      const enriched = await Promise.all(list.map(loadDetail));
      setDetails(enriched);
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setError(msg);
      setDetails([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <section className="cost-panel">
      <header className="panel-head">
        <span className="panel-title">cost</span>
        <span className="panel-sub">workspace + agent budgets via budget::*</span>
        <button
          type="button"
          className="btn-ghost"
          onClick={() => void refresh()}
          disabled={loading}
        >
          {loading ? "refreshing…" : "refresh"}
        </button>
      </header>

      {error ? (
        <p className="panel-error" role="alert">
          {error}
        </p>
      ) : null}

      {!loading && details.length === 0 && !error ? (
        <p className="panel-empty">
          no budgets configured. create one via{" "}
          <code>budget::create</code> on the bus.
        </p>
      ) : null}

      <ul className="budget-list">
        {details.map((d) => {
          const b = d.budget;
          const pct = fmtPct(b.spent_usd, b.ceiling_usd);
          const overBudget = b.spent_usd >= b.ceiling_usd;
          return (
            <li key={b.id} className="budget-row">
              <header className="budget-row-head">
                <span className="budget-id">{b.name ?? b.id}</span>
                <span className="budget-period">{b.period}</span>
                {b.enforced ? (
                  <span className="budget-tag" data-tone="warn">
                    enforced
                  </span>
                ) : null}
                {b.paused ? (
                  <span className="budget-tag" data-tone="muted">
                    paused
                  </span>
                ) : null}
              </header>
              <dl className="budget-grid">
                <div>
                  <dt>spent</dt>
                  <dd data-over={overBudget ? "true" : "false"}>
                    {fmtUsd(b.spent_usd)} <span>/ {fmtUsd(b.ceiling_usd)}</span>
                  </dd>
                </div>
                <div>
                  <dt>utilization</dt>
                  <dd>{pct}</dd>
                </div>
                <div>
                  <dt>resets</dt>
                  <dd>{fmtDate(b.period_resets_at)}</dd>
                </div>
                {d.forecast ? (
                  <>
                    <div>
                      <dt>burn rate</dt>
                      <dd>{fmtUsd(d.forecast.rate_usd_per_day)} /day</dd>
                    </div>
                    <div>
                      <dt>projected (30d)</dt>
                      <dd>{fmtUsd(d.forecast.projected_month_usd)}</dd>
                    </div>
                    <div>
                      <dt>days until breach</dt>
                      <dd>
                        {d.forecast.days_until_breach == null
                          ? "—"
                          : d.forecast.days_until_breach.toFixed(1)}
                      </dd>
                    </div>
                  </>
                ) : null}
              </dl>
              {b.alerts.length > 0 ? (
                <ul className="budget-alerts">
                  {b.alerts.map((a) => (
                    <li key={a.alert_id}>
                      alert · fires at {a.threshold_pct}% →{" "}
                      <code>{a.callback_function_id}</code>
                    </li>
                  ))}
                </ul>
              ) : null}
              {d.error ? (
                <p className="panel-error">usage/forecast · {d.error}</p>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
