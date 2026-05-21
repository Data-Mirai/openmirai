//! Chart.js HTML snippet generators. Each function returns self-contained HTML
//! with an inline Chart.js CDN script tag and canvas element.

use uuid::Uuid;

const CHARTJS_CDN: &str = "https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js";

/// Unique DOM id for a chart canvas.
fn chart_id() -> String {
    format!("chart-{}", &Uuid::new_v4().to_string()[..8])
}

/// Escape a string for safe embedding in a JSON value inside an HTML `<script>` tag.
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

/// Format a slice of f64 as a JSON array literal.
fn values_json(values: &[f64]) -> String {
    let inner: Vec<String> = values.iter().map(|v| format!("{v}")).collect();
    format!("[{}]", inner.join(","))
}

/// Format a string slice as a JSON array of strings.
fn labels_json(labels: &[String]) -> String {
    let inner: Vec<String> = labels
        .iter()
        .map(|l| format!("\"{}\"", json_escape(l)))
        .collect();
    format!("[{}]", inner.join(","))
}

/// Generate a self-contained bar chart HTML snippet.
///
/// `color` is a CSS color string (e.g. `"#a78bfa"`).
pub fn bar_chart(title: &str, labels: &[String], values: &[f64], color: &str) -> String {
    let cid = chart_id();
    let labels_js = labels_json(labels);
    let values_js = values_json(values);
    let title_esc = json_escape(title);
    let color_esc = json_escape(color);
    let show_title = !title.is_empty();

    let mut s = String::new();
    s.push_str(&format!("<script src=\"{}\"></script>\n", CHARTJS_CDN));
    s.push_str("<div style=\"max-width:100%;height:300px\">\n");
    s.push_str(&format!("  <canvas id=\"{}\"></canvas>\n", cid));
    s.push_str("</div>\n");
    s.push_str("<script>\n");
    s.push_str(&format!("new Chart(document.getElementById(\"{}\"), {{\n", cid));
    s.push_str("  type: \"bar\",\n");
    s.push_str(&format!("  data: {{\n    labels: {},\n", labels_js));
    s.push_str(&format!(
        "    datasets: [{{\n      label: \"{}\",\n      data: {},\n",
        title_esc, values_js
    ));
    s.push_str(&format!(
        "      backgroundColor: \"{}40\",\n      borderColor: \"{}\",\n      borderWidth: 2\n    }}]\n",
        color_esc, color_esc
    ));
    s.push_str("  },\n  options: {\n    responsive: true,\n");
    s.push_str(&format!(
        "    plugins: {{ title: {{ display: {}, text: \"{}\", color: \"#e2e8f0\" }} }},\n",
        show_title, title_esc
    ));
    s.push_str("    scales: {\n");
    s.push_str(
        "      x: { ticks: { color: \"#94a3b8\" }, grid: { color: \"#1e293b\" } },\n",
    );
    s.push_str(
        "      y: { ticks: { color: \"#94a3b8\" }, grid: { color: \"#1e293b\" } }\n",
    );
    s.push_str("    }\n  }\n});\n</script>");
    s
}

/// Generate a self-contained line chart HTML snippet.
///
/// Each dataset tuple is `(label, values, color)`.
pub fn line_chart(
    title: &str,
    labels: &[String],
    datasets: &[(String, Vec<f64>, String)],
) -> String {
    let cid = chart_id();
    let labels_js = labels_json(labels);
    let title_esc = json_escape(title);
    let show_title = !title.is_empty();

    let datasets_js: Vec<String> = datasets
        .iter()
        .map(|(label, data, color)| {
            format!(
                "{{label:\"{}\",data:{},borderColor:\"{}\",backgroundColor:\"{}20\",fill:false,tension:0.3,borderWidth:2,pointRadius:3}}",
                json_escape(label),
                values_json(data),
                json_escape(color),
                json_escape(color),
            )
        })
        .collect();

    let mut s = String::new();
    s.push_str(&format!("<script src=\"{}\"></script>\n", CHARTJS_CDN));
    s.push_str("<div style=\"max-width:100%;height:300px\">\n");
    s.push_str(&format!("  <canvas id=\"{}\"></canvas>\n", cid));
    s.push_str("</div>\n");
    s.push_str("<script>\n");
    s.push_str(&format!("new Chart(document.getElementById(\"{}\"), {{\n", cid));
    s.push_str("  type: \"line\",\n");
    s.push_str(&format!("  data: {{\n    labels: {},\n", labels_js));
    s.push_str(&format!("    datasets: [{}]\n", datasets_js.join(",")));
    s.push_str("  },\n  options: {\n    responsive: true,\n");
    s.push_str(&format!(
        "    plugins: {{ title: {{ display: {}, text: \"{}\", color: \"#e2e8f0\" }} }},\n",
        show_title, title_esc
    ));
    s.push_str("    scales: {\n");
    s.push_str(
        "      x: { ticks: { color: \"#94a3b8\" }, grid: { color: \"#1e293b\" } },\n",
    );
    s.push_str(
        "      y: { ticks: { color: \"#94a3b8\" }, grid: { color: \"#1e293b\" } }\n",
    );
    s.push_str("    }\n  }\n});\n</script>");
    s
}

/// Generate a self-contained pie/doughnut chart HTML snippet.
///
/// `colors` should contain one CSS color per value.
pub fn pie_chart(title: &str, labels: &[String], values: &[f64], colors: &[String]) -> String {
    let cid = chart_id();
    let labels_js = labels_json(labels);
    let values_js = values_json(values);
    let title_esc = json_escape(title);
    let show_title = !title.is_empty();

    let bg: Vec<String> = colors
        .iter()
        .map(|c| format!("\"{}80\"", json_escape(c)))
        .collect();
    let border: Vec<String> = colors
        .iter()
        .map(|c| format!("\"{}\"", json_escape(c)))
        .collect();

    let mut s = String::new();
    s.push_str(&format!("<script src=\"{}\"></script>\n", CHARTJS_CDN));
    s.push_str("<div style=\"max-width:100%;height:300px\">\n");
    s.push_str(&format!("  <canvas id=\"{}\"></canvas>\n", cid));
    s.push_str("</div>\n");
    s.push_str("<script>\n");
    s.push_str(&format!("new Chart(document.getElementById(\"{}\"), {{\n", cid));
    s.push_str("  type: \"doughnut\",\n");
    s.push_str(&format!("  data: {{\n    labels: {},\n", labels_js));
    s.push_str(&format!(
        "    datasets: [{{\n      data: {},\n      backgroundColor: [{}],\n      borderColor: [{}],\n      borderWidth: 2\n    }}]\n",
        values_js,
        bg.join(","),
        border.join(","),
    ));
    s.push_str("  },\n  options: {\n    responsive: true,\n");
    s.push_str(&format!(
        "    plugins: {{\n      title: {{ display: {}, text: \"{}\", color: \"#e2e8f0\" }},\n      legend: {{ labels: {{ color: \"#94a3b8\" }} }}\n    }}\n",
        show_title, title_esc
    ));
    s.push_str("  }\n});\n</script>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bar_chart_contains_canvas() {
        let html = bar_chart(
            "Revenue",
            &["Q1".into(), "Q2".into()],
            &[100.0, 200.0],
            "#a78bfa",
        );
        assert!(html.contains("<canvas"));
        assert!(html.contains("chart.js"));
        assert!(html.contains("Revenue"));
        assert!(html.contains("bar"));
    }

    #[test]
    fn test_line_chart_multiple_datasets() {
        let datasets = vec![
            ("A".into(), vec![1.0, 2.0, 3.0], "#22c55e".into()),
            ("B".into(), vec![3.0, 2.0, 1.0], "#ef4444".into()),
        ];
        let html = line_chart(
            "Trends",
            &["Jan".into(), "Feb".into(), "Mar".into()],
            &datasets,
        );
        assert!(html.contains("<canvas"));
        assert!(html.contains("line"));
        assert!(html.contains("#22c55e"));
        assert!(html.contains("#ef4444"));
    }

    #[test]
    fn test_pie_chart_colors() {
        let html = pie_chart(
            "Split",
            &["X".into(), "Y".into()],
            &[60.0, 40.0],
            &["#a78bfa".into(), "#22c55e".into()],
        );
        assert!(html.contains("doughnut"));
        assert!(html.contains("#a78bfa"));
        assert!(html.contains("#22c55e"));
    }

    #[test]
    fn test_bar_chart_empty_title() {
        let html = bar_chart("", &["A".into()], &[1.0], "#000");
        assert!(html.contains("display: false"));
    }
}
