#!/usr/bin/env python3
import glob
import json
import os
import re
from typing import Any, Optional


DASH_DIR = "testing-framework/assets/stack/monitoring/grafana/dashboards"

TITLE_SEP = " — "


METRIC_RE = re.compile(r"\b[a-zA-Z_:][a-zA-Z0-9_:]*\b")


def _collect_exprs(panel: dict[str, Any]) -> list[str]:
    exprs: list[str] = []
    for target in panel.get("targets") or []:
        expr = target.get("expr")
        if isinstance(expr, str) and expr.strip():
            exprs.append(expr.strip())
    return exprs


def _descriptor_from_exprs(title: str, exprs: list[str]) -> Optional[str]:
    if not exprs:
        return None

    all_expr = "\n".join(exprs)

    if "histogram_quantile" in all_expr:
        return "p95 latency"

    if "time() - on() (" in all_expr:
        return "time since last"

    if "consensus_tip_height - consensus_finalized_height" in all_expr:
        return "finalization gap"

    if any("rate(" in e for e in exprs) or any("irate(" in e for e in exprs):
        return "events/sec"

    lower_title = title.lower()
    if "throughput" in lower_title or "tps" in lower_title:
        return "tx/sec"

    if "errors" in lower_title or "fail" in lower_title:
        return "error rate"

    if "peers" in lower_title:
        return "peer count"

    if "connections" in lower_title:
        return "conn count"

    if "queue" in lower_title or "pending" in lower_title:
        return "queue depth"

    # If the title didn't help, infer from obvious metric names.
    metrics = {m for m in METRIC_RE.findall(all_expr) if "_" in m or ":" in m}
    if any(m.endswith("_pending") for m in metrics):
        return "queue depth"
    if any(m.endswith("_height") for m in metrics):
        return "height"
    if any(m.endswith("_slot") for m in metrics):
        return "slot"
    if any(m.endswith("_epoch") for m in metrics):
        return "epoch"
    if any("connections" in m for m in metrics):
        return "conn count"

    return "current"


def _update_panel_title(panel: dict[str, Any]) -> bool:
    if panel.get("type") == "row":
        return False

    title = panel.get("title")
    if not isinstance(title, str) or not title.strip():
        return False

    if TITLE_SEP in title:
        return False

    exprs = _collect_exprs(panel)
    desc = _descriptor_from_exprs(title, exprs)
    if not desc:
        return False

    panel["title"] = f"{title}{TITLE_SEP}{desc}"
    return True


def main() -> int:
    paths = sorted(glob.glob(os.path.join(DASH_DIR, "*.json")))
    if not paths:
        raise SystemExit(f"No dashboards found at {DASH_DIR}")

    changed_files = 0
    changed_panels = 0

    for path in paths:
        with open(path) as f:
            dash = json.load(f)

        changed = False
        for panel in dash.get("panels") or []:
            if _update_panel_title(panel):
                changed = True
                changed_panels += 1

        if changed:
            with open(path, "w") as f:
                json.dump(dash, f, indent=2, sort_keys=False)
                f.write("\n")
            changed_files += 1

    print(f"updated {changed_panels} panels across {changed_files} dashboards")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

