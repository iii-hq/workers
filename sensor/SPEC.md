# iii-sensor

Code quality feedback sensor for the III engine. Analyzes source code structure, computes quality scores, saves baselines, and detects degradation after agent coding sessions. Inspired by Sentrux.

## Functions

### `sensor::scan`

Walks a directory and computes per-file code quality metrics.

**Input:**
```json
{
  "path": "/path/to/scan",
  "extensions": ["rs", "ts"]
}
```

- `path` (required) — directory to scan
- `extensions` (optional) — file extensions to include; defaults to config value

**Output:**
```json
{
  "files": [
    {
      "path": "/path/to/file.rs",
      "language": "rust",
      "line_count": 150,
      "code_lines": 120,
      "complexity": 14,
      "max_depth": 4,
      "function_count": 8,
      "avg_function_length": 15.0,
      "import_count": 5
    }
  ],
  "summary": {
    "total_files": 42,
    "total_lines": 3200,
    "avg_complexity": 8.5,
    "languages": {
      "rust": { "files": 30, "lines": 2400 },
      "typescript": { "files": 12, "lines": 800 }
    }
  }
}
```

**Side effects:** Stores result in `sensor:latest:{path_hash}` state scope.

---

### `sensor::score`

Computes an aggregate quality score (0-100) from scan results using a weighted geometric mean across five dimensions.

**Input (option A — scan inline):**
```json
{ "path": "/path/to/scan" }
```

**Input (option B — pre-computed):**
```json
{ "scan_result": { "files": [...], "summary": {...} } }
```

**Output:**
```json
{
  "score": 78.3,
  "dimensions": {
    "complexity": 85.0,
    "coupling": 72.0,
    "cohesion": 68.5,
    "size": 90.0,
    "duplication": 85.0
  },
  "grade": "C",
  "file_count": 42,
  "timestamp": "2026-04-06T12:00:00Z"
}
```

**Side effects:** Appends score to `sensor:history:{path_hash}` state scope.

---

### `sensor::baseline`

Runs scan + score and saves the result as a named baseline snapshot for later comparison.

**Input:**
```json
{
  "path": "/path/to/scan",
  "label": "pre-session"
}
```

- `label` (optional) — defaults to `"default"`

**Output:**
```json
{
  "baseline_id": "a1b2c3d4:pre-session",
  "score": 78.3,
  "dimensions": { ... },
  "timestamp": "2026-04-06T12:00:00Z",
  "label": "pre-session",
  "file_count": 42,
  "grade": "C"
}
```

**Side effects:** Stores baseline in `sensor:baselines:{path_hash}` state scope under the label key.

---

### `sensor::compare`

Runs a fresh scan + score and compares it against a saved baseline to detect degradation.

**Input:**
```json
{
  "path": "/path/to/scan",
  "baseline_id": "a1b2c3d4:pre-session",
  "label": "pre-session"
}
```

- `baseline_id` or `label` — identifies which baseline to compare against; defaults to `"default"`

**Output:**
```json
{
  "degraded": true,
  "overall_delta": -5.2,
  "dimension_deltas": {
    "complexity": -8.0,
    "coupling": +2.0,
    "cohesion": -3.5,
    "size": -1.0,
    "duplication": 0.0
  },
  "baseline_score": 78.3,
  "current_score": 73.1,
  "degraded_dimensions": ["complexity", "cohesion"],
  "timestamp": "2026-04-06T13:00:00Z"
}
```

A dimension is flagged as degraded when it drops more than `thresholds.degradation_pct` percent from the baseline value.

---

### `sensor::gate`

CI quality gate that returns pass/fail based on absolute score thresholds and degradation limits.

**Input:**
```json
{
  "path": "/path/to/scan",
  "min_score": 60,
  "max_degradation_pct": 10
}
```

- `min_score` (optional) — defaults to config `thresholds.min_score` (60)
- `max_degradation_pct` (optional) — defaults to config `thresholds.degradation_pct` (10)

**Output:**
```json
{
  "passed": false,
  "score": 58.2,
  "grade": "F",
  "reason": "score 58.2 below minimum 60.0",
  "details": {
    "dimensions": { ... },
    "file_count": 42,
    "min_score": 60,
    "max_degradation_pct": 10
  }
}
```

Gate checks both absolute score and degradation against the `default` baseline (if one exists).

---

### `sensor::history`

Retrieves historical quality scores and computes trend direction.

**Input:**
```json
{
  "path": "/path/to/scan",
  "limit": 20
}
```

**Output:**
```json
{
  "scores": [
    { "score": 78.3, "dimensions": {...}, "timestamp": "...", "grade": "C" },
    { "score": 75.1, "dimensions": {...}, "timestamp": "...", "grade": "C" }
  ],
  "total_entries": 15,
  "trend": "degrading"
}
```

Trend is computed from the oldest to newest score in the window:
- `improving` — score increased by more than 2 points
- `degrading` — score decreased by more than 2 points
- `stable` — within 2 points

---

## Scoring Methodology

Each file contributes to five dimensions, each scored 0-100:

| Dimension | What it measures | Penalty |
|-----------|-----------------|---------|
| **Complexity** | Branching keywords per 100 code lines | >5/100 lines, -5 points per extra |
| **Coupling** | Average import count per file | >5 imports, -3 points per extra |
| **Cohesion** | Function count proportional to file size | Deviation from ideal ratio (1 fn per 20 lines) |
| **Size** | Average code lines per file | >100 lines, -0.2 points per extra |
| **Duplication** | Similar line patterns | Placeholder (fixed at 85 for v1) |

The final score uses a **weighted power mean** (approximation of Nash Social Welfare / geometric mean) so that gaming one dimension at the expense of others is penalized:

```
score = complexity^0.25 * coupling^0.25 * cohesion^0.20 * size^0.15 * duplication^0.15
```

Grades: A (90+), B (80-89), C (70-79), D (60-69), F (<60)

---

## State Scopes

| Scope | Key | Value |
|-------|-----|-------|
| `sensor:baselines:{path_hash}` | label string | Baseline snapshot JSON |
| `sensor:history:{path_hash}` | `"scores"` | Array of score results |
| `sensor:latest:{path_hash}` | `"scan"` | Most recent scan result |

`path_hash` is a deterministic hex hash of the scanned directory path using `DefaultHasher`.

---

## Triggers

Each function is registered as an iii-engine function callable via `iii.trigger()`:

- `sensor::scan` — HTTP trigger
- `sensor::score` — HTTP trigger
- `sensor::baseline` — HTTP trigger
- `sensor::compare` — HTTP trigger
- `sensor::gate` — HTTP trigger
- `sensor::history` — HTTP trigger

Queue trigger on `sensor.scan.requested` topic is planned for async scan jobs (v2).

---

## Configuration

`config.yaml`:

```yaml
scan_extensions: ["rs", "ts", "py", "js", "go"]
max_file_size_kb: 512
score_weights:
  complexity: 0.25
  coupling: 0.25
  cohesion: 0.20
  size: 0.15
  duplication: 0.15
thresholds:
  degradation_pct: 10.0
  min_score: 60.0
```

---

## CLI

```
iii-sensor --config ./config.yaml --url ws://127.0.0.1:49134
iii-sensor --manifest   # output module manifest JSON and exit
```
