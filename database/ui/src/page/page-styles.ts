/**
 * The database page's own CSS, rendered once as a scoped <style> at the top
 * of `DatabasePage`. Injected UI can't lean on the console's Tailwind
 * utility output, so the source's utility classes are ported here as real
 * rules — every one scoped under `.db-page` and coloured from the console's
 * design tokens (`var(--color-*)`) so light/dark theming is free.
 */

export const PAGE_CSS = `
.db-page {
  font-family: var(--font-mono, ui-monospace, monospace);
  color: var(--color-ink);
  padding: 20px 24px;
  box-sizing: border-box;
}
.db-page *, .db-page *::before, .db-page *::after { box-sizing: border-box; }

.db-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--color-rule);
}
.db-title {
  font-size: 16px;
  font-weight: 600;
  letter-spacing: -0.01em;
  text-transform: lowercase;
  color: var(--color-ink);
}
.db-sub {
  font-size: 12px;
  color: var(--color-ink-faint);
  margin-top: 2px;
  text-transform: lowercase;
  word-break: break-all;
}
.db-controls { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }

.db-modes {
  display: inline-flex;
  border: 1px solid var(--color-rule);
}
.db-mode {
  appearance: none;
  background: transparent;
  border: 0;
  color: var(--color-ink-faint);
  font: inherit;
  font-size: 12px;
  text-transform: lowercase;
  padding: 3px 12px;
  cursor: pointer;
}
.db-mode + .db-mode { border-left: 1px solid var(--color-rule); }
.db-mode:hover { color: var(--color-ink); }
.db-mode.active { background: var(--color-accent); color: var(--color-accent-fg); }

.db-driver-badge {
  border: 1px solid var(--color-accent);
  color: var(--color-accent);
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  padding: 1px 8px;
}

.db-body {
  display: grid;
  grid-template-columns: minmax(200px, 240px) minmax(0, 1fr);
  gap: 16px;
  align-items: start;
  margin-top: 16px;
}

.db-aside {
  border: 1px solid var(--color-rule);
  background: var(--color-bg);
  max-height: 74vh;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}
.db-aside-head {
  position: sticky;
  top: 0;
  background: var(--color-bg);
  padding: 8px 12px;
  border-bottom: 1px solid var(--color-rule-2);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  color: var(--color-ink-ghost);
}
.db-aside-body { flex: 1; padding: 4px 0; }

.db-panel {
  border: 1px solid var(--color-rule);
  background: var(--color-bg);
  min-height: 360px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.db-msg {
  padding: 10px 12px;
  font-size: 12px;
  color: var(--color-ink-ghost);
  text-transform: lowercase;
}
.db-msg.alert { color: var(--color-alert); text-transform: none; word-break: break-all; }
.db-pulse { animation: db-page-pulse 1.6s ease-in-out infinite; }
@keyframes db-page-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.45; } }

.db-pad { padding: 16px; }
.db-skel { display: flex; flex-direction: column; gap: 8px; padding: 12px; }

/* ---- schema tree ---- */
.db-tree-grouphead {
  padding: 12px 12px 4px;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  color: var(--color-ink-ghost);
}
.db-tree ul { list-style: none; margin: 0; padding: 0; }
.db-tree-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding-right: 8px;
  border-left: 2px solid transparent;
}
.db-tree-row:hover { background: color-mix(in oklab, var(--color-paper-2) 60%, transparent); }
.db-tree-row.active { background: var(--color-paper-2); border-left-color: var(--color-accent); }
.db-tree-toggle {
  appearance: none; background: transparent; border: 0; cursor: pointer;
  padding: 6px 0 6px 8px; color: var(--color-ink-ghost); display: inline-flex;
}
.db-tree-toggle:hover { color: var(--color-ink); }
.db-tree-name {
  appearance: none; background: transparent; border: 0; cursor: pointer;
  display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0;
  padding: 6px 0; font: inherit; font-size: 12.5px; text-align: left;
  color: var(--color-ink-faint);
}
.db-tree-name:hover { color: var(--color-ink); }
.db-tree-row.active .db-tree-name { color: var(--color-ink); }
.db-trunc { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.db-cols { padding-bottom: 4px; }
.db-col {
  display: flex; align-items: center; gap: 6px;
  padding: 3px 12px 3px 36px; font-size: 11.5px;
}
.db-col-name { color: var(--color-ink-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.db-col-type { color: var(--color-ink-ghost); text-transform: lowercase; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.db-idx-head {
  padding: 6px 12px 2px 36px; font-size: 9.5px;
  text-transform: uppercase; letter-spacing: 0.1em; color: var(--color-ink-ghost);
}
.db-idx { display: flex; align-items: center; gap: 6px; padding: 3px 12px 3px 36px; font-size: 11px; }
.db-idx-name { color: var(--color-ink-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.db-idx-unique { color: var(--color-accent); font-size: 9.5px; text-transform: uppercase; letter-spacing: 0.06em; }
.db-tree-msg { padding: 4px 12px 4px 36px; font-size: 11px; color: var(--color-ink-ghost); }
.db-tree-msg.alert { color: var(--color-alert); }

/* ---- result grid ---- */
.db-grid-wrap { overflow-x: auto; }
.db-grid { width: 100%; border-collapse: collapse; }
.db-grid thead.sticky th { position: sticky; top: 0; z-index: 10; }
.db-grid th {
  padding: 8px 12px; font-size: 11px; white-space: nowrap;
  background: var(--color-paper-2); border-bottom: 1px solid var(--color-rule);
  text-align: left; font-weight: 500;
}
.db-grid th.num { text-align: right; }
.db-grid th .colname { color: var(--color-ink-faint); font-weight: 500; }
.db-grid th .coltype { color: var(--color-ink-ghost); font-weight: 400; text-transform: lowercase; }
.db-grid th .sorter {
  appearance: none; background: transparent; border: 0; cursor: pointer;
  display: inline-flex; align-items: center; gap: 6px; font: inherit; color: inherit;
}
.db-grid th.num .sorter { flex-direction: row-reverse; }
.db-grid th .sorter:hover .colname { color: var(--color-ink); }
.db-grid th .static { display: inline-flex; align-items: center; gap: 6px; }
.db-grid td {
  padding: 6px 12px; font-size: 12.5px; vertical-align: top; white-space: nowrap;
  max-width: 28rem; overflow: hidden; text-overflow: ellipsis;
  border-bottom: 1px solid var(--color-rule-2);
}
.db-grid td.num { text-align: right; }
.db-grid tbody tr:last-child td { border-bottom: 0; }
.db-grid tbody tr.clickable { cursor: pointer; }
.db-grid tbody tr:hover { background: color-mix(in oklab, var(--color-paper-2) 60%, transparent); }
.db-grid tbody tr.selected { background: var(--color-paper-2); }
.db-grid td .copycell {
  appearance: none; background: transparent; border: 0; padding: 0; margin: 0;
  font: inherit; color: inherit; cursor: copy; text-align: left;
  max-width: 100%; overflow: hidden; text-overflow: ellipsis;
}
.db-cell-null { color: var(--color-ink-ghost); font-style: italic; }
.db-cell-bool-true { color: var(--color-accent); }
.db-cell-bool-false { color: var(--color-warn); }
.db-cell-num { color: var(--color-ink); font-variant-numeric: tabular-nums; }
.db-cell-json { color: var(--color-ink-faint); }
.db-cell-str { color: var(--color-ink); }
.db-cell-date { color: var(--color-ink-faint); }
.db-copied { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; color: var(--color-accent); }

.db-grid-foot {
  display: flex; align-items: center; gap: 8px;
  padding: 6px 12px; border-top: 1px solid var(--color-rule-2);
  font-size: 11px; color: var(--color-ink-faint);
}
.db-linkish {
  appearance: none; background: transparent; border: 0; padding: 0; cursor: pointer;
  font: inherit; color: var(--color-accent);
}
.db-linkish:hover { text-decoration: underline; }
.db-grid-empty { padding: 12px; font-size: 12.5px; color: var(--color-ink-ghost); }

/* ---- table data panel ---- */
.db-data { display: flex; min-height: 0; min-width: 0; width: 100%; }
.db-data-main { flex: 1; display: flex; flex-direction: column; min-width: 0; }
.db-data-bar {
  display: flex; flex-wrap: wrap; align-items: center; gap: 12px;
  padding: 8px 16px; border-bottom: 1px solid var(--color-rule);
  font-size: 11px; color: var(--color-ink-faint);
}
.db-data-bar .name { color: var(--color-ink); font-weight: 500; }
.db-data-bar .spacer { margin-left: auto; }
.db-data-scroll { overflow: auto; max-height: 60vh; }
.db-data-foot { border-top: 1px solid var(--color-rule); padding: 8px 16px; }

/* ---- pagination ---- */
.db-pager {
  display: flex; align-items: center; justify-content: space-between; gap: 16px;
  font-size: 12px; color: var(--color-ink-faint); text-transform: lowercase;
}
.db-pager-group { display: flex; align-items: center; gap: 8px; font-variant-numeric: tabular-nums; }
.db-pager-cap { text-transform: uppercase; letter-spacing: 0.06em; font-size: 11px; }
.db-pager select {
  border: 1px solid var(--color-rule); background: var(--color-bg); color: var(--color-ink);
  font: inherit; font-size: 12px; padding: 2px 6px; border-radius: 0;
}
.db-pager select:focus { outline: none; border-color: var(--color-accent); }

/* ---- row inspector ---- */
.db-rowdetail {
  width: 320px; flex-shrink: 0; border-left: 1px solid var(--color-rule);
  overflow-y: auto; max-height: 60vh;
}
.db-rowdetail-head {
  position: sticky; top: 0; background: var(--color-paper-2);
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 12px; border-bottom: 1px solid var(--color-rule);
}
.db-rowdetail-head .cap { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--color-ink-faint); }
.db-field { padding: 8px 12px; border-bottom: 1px solid var(--color-rule-2); }
.db-field-head { display: flex; align-items: center; gap: 6px; }
.db-field-name { font-size: 11px; font-weight: 500; color: var(--color-ink-faint); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.db-field-type { font-size: 10px; color: var(--color-ink-ghost); text-transform: lowercase; }
.db-field-copy { appearance: none; background: transparent; border: 0; padding: 0; cursor: pointer; color: var(--color-ink-ghost); display: inline-flex; }
.db-field-copy:hover { color: var(--color-ink); }
.db-field-copy.copied { color: var(--color-accent); }
.db-field-value { margin-top: 4px; font-size: 12px; word-break: break-all; }
.db-field-value .null { color: var(--color-ink-ghost); font-style: italic; }
.db-field-value .bool-true { color: var(--color-accent); }
.db-field-value .bool-false { color: var(--color-warn); }
.db-field-value .plain { color: var(--color-ink); white-space: pre-wrap; }
.db-field-fk { margin-top: 4px; font-size: 10px; color: var(--color-ink-ghost); }
.db-icon-btn {
  appearance: none; background: transparent; border: 0; cursor: pointer;
  color: var(--color-ink-ghost); display: inline-flex; padding: 2px;
}
.db-icon-btn:hover { color: var(--color-ink); }
.db-icon-btn:disabled { opacity: 0.4; cursor: default; }

/* ---- sql panel ---- */
.db-sql { display: flex; flex-direction: column; min-height: 0; min-width: 0; width: 100%; }
.db-sql-top { border-bottom: 1px solid var(--color-rule); }
.db-sql-editor-wrap { max-height: 240px; overflow: auto; background: var(--color-bg); }
.db-sql-code { min-height: 104px; }
.db-sql-actions {
  display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
  padding: 8px 16px; border-top: 1px solid var(--color-rule-2); background: var(--color-paper-2);
}
.db-sql-warn { font-size: 11px; color: var(--color-warn); }
.db-sql-meta { margin-left: auto; font-size: 11px; color: var(--color-ink-faint); font-variant-numeric: tabular-nums; }
.db-sql-history { border-top: 1px solid var(--color-rule-2); max-height: 160px; overflow-y: auto; }
.db-sql-history-row {
  display: flex; align-items: center; gap: 8px; padding: 4px 16px;
  border-bottom: 1px solid var(--color-rule-2);
}
.db-sql-history-row:last-child { border-bottom: 0; }
.db-sql-history-row:hover { background: color-mix(in oklab, var(--color-paper-2) 60%, transparent); }
.db-sql-history-pick {
  appearance: none; background: transparent; border: 0; cursor: pointer; flex: 1; min-width: 0;
  text-align: left; font: inherit; font-size: 11.5px; color: var(--color-ink-faint);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.db-sql-history-pick:hover { color: var(--color-ink); }
.db-sql-results { flex: 1; overflow: auto; max-height: 55vh; min-height: 0; }
.db-sql-placeholder { padding: 16px; font-size: 12px; color: var(--color-ink-ghost); text-transform: lowercase; }

@media (max-width: 720px) {
  .db-body { grid-template-columns: minmax(0, 1fr); }
  .db-data { flex-direction: column; }
  .db-rowdetail { width: auto; border-left: 0; border-top: 1px solid var(--color-rule); }
}
`
