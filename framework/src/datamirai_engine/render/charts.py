"""Chart helpers — generate Chart.js chart configurations as HTML snippets.

Each helper returns a self-contained <canvas> + <script> block that renders
a chart using Chart.js (loaded inline in the HTML document by render_rich).
"""

from __future__ import annotations

import json
import uuid


def _chart_id() -> str:
    return f"chart-{uuid.uuid4().hex[:8]}"


def bar_chart(
    labels: list[str],
    datasets: list[dict],
    title: str = "",
    height: int = 300,
) -> str:
    """Generate a bar chart HTML snippet.

    datasets: list of {label: str, data: list[number], color?: str}
    """
    cid = _chart_id()
    ds = []
    colors = ["#a78bfa", "#22c55e", "#06b6d4", "#f59e0b", "#ef4444", "#ec4899"]
    for i, d in enumerate(datasets):
        color = d.get("color", colors[i % len(colors)])
        ds.append({
            "label": d.get("label", f"Serie {i+1}"),
            "data": d["data"],
            "backgroundColor": color + "40",
            "borderColor": color,
            "borderWidth": 2,
        })
    config = {
        "type": "bar",
        "data": {"labels": labels, "datasets": ds},
        "options": {
            "responsive": True,
            "plugins": {"title": {"display": bool(title), "text": title, "color": "#e2e8f0"}},
            "scales": {
                "x": {"ticks": {"color": "#94a3b8"}, "grid": {"color": "#1e293b"}},
                "y": {"ticks": {"color": "#94a3b8"}, "grid": {"color": "#1e293b"}},
            },
        },
    }
    return f'<div style="max-width:100%;height:{height}px"><canvas id="{cid}"></canvas></div>\n<script>new Chart(document.getElementById("{cid}"),{json.dumps(config)});</script>'


def line_chart(
    labels: list[str],
    datasets: list[dict],
    title: str = "",
    height: int = 300,
) -> str:
    """Generate a line chart HTML snippet."""
    cid = _chart_id()
    ds = []
    colors = ["#a78bfa", "#22c55e", "#06b6d4", "#f59e0b", "#ef4444", "#ec4899"]
    for i, d in enumerate(datasets):
        color = d.get("color", colors[i % len(colors)])
        ds.append({
            "label": d.get("label", f"Serie {i+1}"),
            "data": d["data"],
            "borderColor": color,
            "backgroundColor": color + "20",
            "fill": d.get("fill", False),
            "tension": 0.3,
            "borderWidth": 2,
            "pointRadius": 3,
        })
    config = {
        "type": "line",
        "data": {"labels": labels, "datasets": ds},
        "options": {
            "responsive": True,
            "plugins": {"title": {"display": bool(title), "text": title, "color": "#e2e8f0"}},
            "scales": {
                "x": {"ticks": {"color": "#94a3b8"}, "grid": {"color": "#1e293b"}},
                "y": {"ticks": {"color": "#94a3b8"}, "grid": {"color": "#1e293b"}},
            },
        },
    }
    return f'<div style="max-width:100%;height:{height}px"><canvas id="{cid}"></canvas></div>\n<script>new Chart(document.getElementById("{cid}"),{json.dumps(config)});</script>'


def pie_chart(
    labels: list[str],
    data: list[float],
    title: str = "",
    height: int = 300,
) -> str:
    """Generate a pie/doughnut chart HTML snippet."""
    cid = _chart_id()
    colors = ["#a78bfa", "#22c55e", "#06b6d4", "#f59e0b", "#ef4444", "#ec4899", "#8b5cf6", "#14b8a6"]
    bg = [colors[i % len(colors)] + "80" for i in range(len(data))]
    border = [colors[i % len(colors)] for i in range(len(data))]
    config = {
        "type": "doughnut",
        "data": {
            "labels": labels,
            "datasets": [{"data": data, "backgroundColor": bg, "borderColor": border, "borderWidth": 2}],
        },
        "options": {
            "responsive": True,
            "plugins": {
                "title": {"display": bool(title), "text": title, "color": "#e2e8f0"},
                "legend": {"labels": {"color": "#94a3b8"}},
            },
        },
    }
    return f'<div style="max-width:100%;height:{height}px"><canvas id="{cid}"></canvas></div>\n<script>new Chart(document.getElementById("{cid}"),{json.dumps(config)});</script>'


# CDN URL for Chart.js (loaded inline)
CHARTJS_CDN = "https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js"

# Minimal inline loader script
CHARTJS_LOADER = f'<script src="{CHARTJS_CDN}"></script>'
