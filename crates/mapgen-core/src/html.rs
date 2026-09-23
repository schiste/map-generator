//! Self-contained HTML page with the map inline and live colour controls.

use std::fmt::Write;

use crate::svg::escape;
use crate::theme::{Theme, COLOR_SLOTS};

const LABELS: [&str; 10] = [
    "Background",
    "Water",
    "Land",
    "Neighbours",
    "Borders",
    "Outline",
    "Neighbour borders",
    "Lake shores",
    "Disputed borders",
    "Labels",
];

/// Wraps an SVG rendered with `css_vars = true` in an interactive page:
/// colour pickers for every theme slot, theme presets, hover names, and a
/// "Download SVG" button that bakes the chosen colours into the file.
pub fn html_page(svg: &str, title: &str, theme: &Theme, download_name: &str) -> String {
    let svg = svg.split_once("?>\n").map_or(svg, |(_, rest)| rest);
    let mut themes = String::from("{");
    for (i, name) in Theme::NAMES.iter().enumerate() {
        let t = Theme::builtin(name).expect("built-in theme");
        let _ = write!(
            themes,
            "{}{}:{}",
            if i > 0 { "," } else { "" },
            js(name),
            colors_json(&t)
        );
    }
    themes.push('}');
    let options: String = Theme::NAMES
        .iter()
        .map(|n| format!("<option value=\"{n}\">{n}</option>"))
        .collect();
    let slots: Vec<String> = COLOR_SLOTS.iter().map(|s| js(s)).collect();
    let labels: Vec<String> = LABELS.iter().map(|s| js(s)).collect();

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title_html}</title>
<style>
:root {{ font: 14px/1.4 system-ui, -apple-system, "Segoe UI", sans-serif; color: #1d2330; background: #f4f5f7; }}
body {{ margin: 0; display: grid; grid-template-columns: 280px minmax(0, 1fr); min-height: 100vh; }}
aside {{ padding: 16px; background: #fff; border-right: 1px solid #e3e5e8; display: flex; flex-direction: column; gap: 10px; }}
h1 {{ font-size: 15px; margin: 0 0 4px; }}
.row {{ display: grid; grid-template-columns: 1fr 34px 96px; align-items: center; gap: 6px; }}
input[type=color] {{ width: 34px; height: 26px; padding: 0; border: 1px solid #cfd3d8; border-radius: 4px; background: none; }}
input[type=text] {{ width: 100%; box-sizing: border-box; font: 12px ui-monospace, monospace; padding: 4px 6px; border: 1px solid #cfd3d8; border-radius: 4px; }}
select, button {{ font: inherit; padding: 6px 10px; border-radius: 6px; border: 1px solid #cfd3d8; background: #fff; cursor: pointer; }}
button.primary {{ background: #1d2330; color: #fff; border-color: #1d2330; }}
.hint {{ color: #667; font-size: 12px; margin: 4px 0 0; }}
code {{ font-size: 11px; }}
main {{ padding: 16px; display: flex; flex-direction: column; gap: 8px; min-width: 0; }}
#map svg {{ width: 100%; height: auto; display: block; box-shadow: 0 1px 3px rgba(0,0,0,.15); }}
#map .mg-land:hover {{ fill: #ffcf4d; }}
#info {{ min-height: 1.4em; color: #445; }}
@media (max-width: 720px) {{ body {{ grid-template-columns: 1fr; }} aside {{ border-right: 0; border-bottom: 1px solid #e3e5e8; }} }}
</style>
</head>
<body>
<aside>
<h1>{title_html}</h1>
<label class="row" style="grid-template-columns: 1fr 136px">Theme <select id="theme">{options}</select></label>
<div id="controls"></div>
<button class="primary" id="download">Download SVG</button>
<button id="reset">Reset colours</button>
<p class="hint">Any CSS colour works in the text fields (<code>none</code>, <code>steelblue</code>, <code>rgb(…)</code>). When embedding the SVG inline, set <code>--mg-water</code>, <code>--mg-land</code>… on a parent element to recolour it.</p>
</aside>
<main>
<div id="info">Hover a region to see its name.</div>
<div id="map">
{svg}</div>
</main>
<script>
const SLOTS = [{slots}];
const LABELS = [{labels}];
const DEFAULTS = {defaults};
const THEMES = {themes};
const FILE = {file};
const svg = document.querySelector('#map svg');
const current = {{ ...DEFAULTS }};
const controls = document.getElementById('controls');
const hex = v => /^#[0-9a-f]{{6}}$/i.test(v) ? v : /^#[0-9a-f]{{3}}$/i.test(v) ? '#' + [...v.slice(1)].map(c => c + c).join('') : null;

function set(slot, value) {{
  current[slot] = value;
  svg.style.setProperty('--mg-' + slot, value);
  const row = controls.querySelector(`[data-slot="${{slot}}"]`);
  row.querySelector('input[type=text]').value = value;
  const h = hex(value);
  if (h) row.querySelector('input[type=color]').value = h;
}}

SLOTS.forEach((slot, i) => {{
  const row = document.createElement('label');
  row.className = 'row';
  row.dataset.slot = slot;
  row.innerHTML = `<span>${{LABELS[i]}}</span><input type="color"><input type="text" spellcheck="false">`;
  const [pick, text] = row.querySelectorAll('input');
  pick.addEventListener('input', () => set(slot, pick.value));
  text.addEventListener('change', () => set(slot, text.value.trim() || DEFAULTS[slot]));
  controls.append(row);
  set(slot, DEFAULTS[slot]);
}});

document.getElementById('theme').addEventListener('change', e => {{
  const t = THEMES[e.target.value];
  SLOTS.forEach(s => set(s, t[s]));
}});
document.getElementById('reset').addEventListener('click', () => SLOTS.forEach(s => set(s, DEFAULTS[s])));

const info = document.getElementById('info');
svg.addEventListener('mouseover', e => {{
  const p = e.target.closest('path[data-name]');
  if (p) info.textContent = `${{p.dataset.name}} — #${{p.id}}`;
}});

document.getElementById('download').addEventListener('click', () => {{
  const clone = svg.cloneNode(true);
  clone.removeAttribute('style');
  const style = clone.querySelector('style');
  let css = style.textContent;
  SLOTS.forEach(s => {{ css = css.split(`var(--mg-${{s}},${{DEFAULTS[s]}})`).join(current[s]); }});
  style.textContent = css;
  const xml = '<?xml version="1.0" encoding="UTF-8"?>\n' + new XMLSerializer().serializeToString(clone) + '\n';
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([xml], {{ type: 'image/svg+xml' }}));
  a.download = FILE;
  a.click();
  URL.revokeObjectURL(a.href);
}});
</script>
</body>
</html>
"##,
        title_html = escape(title),
        slots = slots.join(","),
        labels = labels.join(","),
        defaults = colors_json(theme),
        file = js(download_name),
    )
}

fn colors_json(t: &Theme) -> String {
    let fields: Vec<String> = t
        .colors()
        .iter()
        .map(|(slot, c)| format!("{}:{}", js(slot), js(c.as_str())))
        .collect();
    format!("{{{}}}", fields.join(","))
}

/// A JS string literal that is also safe inside `<script>`.
fn js(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_strings_cannot_close_script() {
        assert_eq!(js("</script>"), "\"\\u003c/script\\u003e\"");
        assert_eq!(js("a\"b\\"), "\"a\\\"b\\\\\"");
    }
}
