"""
Generate autoharness progress chart from experiment history.

Chart style:
- Multi-dataset overlay on one chart
- Percentage Y-axis (0-100%)
- Step lines for running best per dataset
- Kept (large dots) vs discarded (small faded dots)
- Labels on kept experiments

Usage:
    python plot_progress.py --tag apr06
    python plot_progress.py --tag apr06 --tag terminal-run --output progress.png
    python plot_progress.py --tag apr06 --api http://remote-host:3111
"""

import argparse
import json
import urllib.request

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.ticker as mtick
import numpy as np


PALETTE = [
    {"line": "#1a9e76", "kept": "#1a9e76", "discard": "#a6dbb5", "name_suffix": ""},
    {"line": "#5a6acf", "kept": "#5a6acf", "discard": "#b3b9e8", "name_suffix": ""},
    {"line": "#e07b39", "kept": "#e07b39", "discard": "#f0c9a6", "name_suffix": ""},
    {"line": "#d45087", "kept": "#d45087", "discard": "#eab0c9", "name_suffix": ""},
]


def fetch_json(url, data=None):
    body = json.dumps(data).encode() if data else None
    headers = {"Content-Type": "application/json"} if body else {}
    req = urllib.request.Request(url, data=body, headers=headers, method="POST" if body else "GET")
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def plot_tag(ax, experiments, tag, color, offset=0):
    experiments.sort(key=lambda e: e.get("started_at", ""))

    kept_x, kept_y, kept_labels = [], [], []
    discard_x, discard_y = [], []
    best_x, best_y = [], []
    running_best = None

    for i, exp in enumerate(experiments):
        idx = i + offset
        status = exp.get("status", "running")
        score = exp.get("aggregate_score", 0) * 100

        if status in ("crash", "running"):
            continue

        if status == "keep":
            kept_x.append(idx)
            kept_y.append(score)
            desc = exp.get("description", "")
            if len(desc) > 35:
                desc = desc[:32] + "..."
            kept_labels.append(desc)
            running_best = score
        else:
            discard_x.append(idx)
            discard_y.append(score)

        if running_best is not None:
            best_x.append(idx)
            best_y.append(running_best)

    if best_x:
        order = np.argsort(best_x)
        bx = np.array(best_x)[order]
        by = np.array(best_y)[order]
        bx = np.append(bx, bx[-1] + 2)
        by = np.append(by, by[-1])
        ax.step(bx, by, where="post", color=color["line"], linewidth=2, alpha=0.9,
                label=f"{tag} \u2014 running best", zorder=3)

    if kept_x:
        ax.scatter(kept_x, kept_y, c=color["kept"], s=70, edgecolors="white",
                   linewidth=0.8, label=f"{tag} \u2014 kept", zorder=4)
        for x, y, label in zip(kept_x, kept_y, kept_labels):
            ax.annotate(
                label, (x, y),
                textcoords="offset points", xytext=(6, 6),
                fontsize=6.5, color=color["kept"], alpha=0.9,
                bbox=dict(boxstyle="round,pad=0.2", fc="white", ec="none", alpha=0.7),
            )

    if discard_x:
        ax.scatter(discard_x, discard_y, c=color["discard"], s=25, alpha=0.5,
                   label=f"{tag} \u2014 discarded", zorder=2)

    return len([e for e in experiments if e["status"] != "running"])


def main():
    parser = argparse.ArgumentParser(description="Plot autoharness progress")
    parser.add_argument("--tag", required=True, action="append", help="Experiment run tag (can specify multiple)")
    parser.add_argument("--api", default="http://localhost:3111", help="iii-engine REST API")
    parser.add_argument("--output", default="progress.png", help="Output image path")
    args = parser.parse_args()

    fig, ax = plt.subplots(figsize=(16, 8))

    total_exps = 0
    tag_labels = []

    for i, tag in enumerate(args.tag):
        color = PALETTE[i % len(PALETTE)]
        data = fetch_json(f"{args.api}/api/experiment/history", {"tag": tag, "limit": 500})
        payload = data.get("body", data) if isinstance(data, dict) else data
        if isinstance(payload, str):
            payload = json.loads(payload)
        experiments = payload if isinstance(payload, list) else payload.get("experiments", [])

        if not experiments:
            print(f"No experiments found for tag '{tag}'")
            continue

        n = plot_tag(ax, experiments, tag, color, offset=0)
        total_exps += n
        tag_labels.append(tag)

    title_datasets = " and ".join(t.replace("-", " ").title() for t in tag_labels) if tag_labels else "No Data"
    ax.set_title(f"Autoharness Progress: {title_datasets}", fontsize=16, fontweight="bold", pad=15)

    ax.set_xlabel("Experiment #", fontsize=13)
    ax.set_ylabel("Score", fontsize=13)
    ax.yaxis.set_major_formatter(mtick.PercentFormatter())
    ax.set_ylim(0, 105)
    ax.grid(True, alpha=0.25, linewidth=0.5)
    ax.axhline(y=100, color="#cccccc", linestyle="-", alpha=0.4, linewidth=0.8)

    ax.legend(loc="lower right", fontsize=9, framealpha=0.9, ncol=min(len(tag_labels), 2) * 3 or 1)

    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)

    fig.tight_layout()
    fig.savefig(args.output, dpi=150, bbox_inches="tight", facecolor="white")
    print(f"Saved progress chart to {args.output}")
    print(f"  Tags: {', '.join(tag_labels)}")
    print(f"  Total experiments: {total_exps}")


if __name__ == "__main__":
    main()
