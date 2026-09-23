"""Writes Natural Earth's points of view as differences from the de facto
countries, for the playground (a few hundred KB each instead of 24 MB):

  python3 scripts/worldview-diffs.py data/ne_10m_admin_0.geojson OUT_DIR

reads `ne_10m_admin_0_<view>.geojson` next to the countries (fetched with
`scripts/fetch-data.sh ne-worldview <VIEW>`) and writes, per view,
`ne_10m_admin_0_countries.<VIEW>.json`: {"removed": [indices of the de facto
features the view drops or changes], "added": [its own versions of them]},
and `worldviews.json`, the list of views written.
"""
import json, os, re, sys

base_path, out = sys.argv[1:3]
folder, stem = os.path.dirname(base_path), os.path.basename(base_path)[: -len(".geojson")]
# A feature is the same when its code, name and shape are: views also
# differ in columns maps don't use (FCLASS_*...), which would bloat the diff.
key = lambda f: json.dumps(
    [f["properties"].get(k) for k in ("ADM0_A3", "NAME", "ISO_A2_EH", "WIKIDATAID")] + [f["geometry"]],
    ensure_ascii=False,
)
base = json.load(open(base_path, encoding="utf-8"))["features"]
base_keys = [key(f) for f in base]
views = []
for name in sorted(os.listdir(folder)):
    m = re.fullmatch(re.escape(stem) + r"_([a-z]{3})\.geojson", name)
    if not m:
        continue
    view = m.group(1).upper()
    features = json.load(open(os.path.join(folder, name), encoding="utf-8"))["features"]
    keys = {key(f) for f in features}
    removed = [i for i, k in enumerate(base_keys) if k not in keys]
    known = set(base_keys)
    added = [f for f in features if key(f) not in known]
    with open(os.path.join(out, f"ne_10m_admin_0_countries.{view}.json"), "w", encoding="utf-8") as f:
        json.dump({"removed": removed, "added": added}, f, ensure_ascii=False, separators=(",", ":"))
    views.append(view)
with open(os.path.join(out, "worldviews.json"), "w", encoding="utf-8") as f:
    json.dump(views, f)
print(f"worldviews: {len(views)} ({', '.join(views)})")
