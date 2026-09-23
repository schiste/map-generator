# Uploading maps to Wikimedia Commons

`mapgen` doesn't upload or edit anything. It produces what upload tools need: predictable file
names, a manifest with each map's SHA-1 and description fields, and optional description
pages. A read-only script tells which maps are already on Commons.

## 1. Render the set

```sh
mapgen batch -i data/ne_10m_admin_1.geojson --dataset ne-admin1 --context data/ne_10m_admin_0.geojson \
  --out-dir out/ --name-template "Blank map of {name} ({level}, {year}).svg" --boundary-year 2024 \
  --manifest out/manifest.csv --wikitext \
  --author "[[User:Example|Example]]" --date 2026-09-23 --license "{{self|cc-by-4.0}}" \
  --categories "Blank maps of {name},Maps made with map-generator"
```

- **Names.** `--name-template` placeholders: `{code}` (region code), `{name}` (its name in
  `--context`, else the code), `{level}` (`ADM1`…, from the geoBoundaries sidecar or the
  Natural Earth preset), `{year}` (boundary year), `{stem}` (input file name). Characters
  Commons forbids in titles (`# < > [ ] | { } / :`) become `-`. Two maps with the same
  name stop the run. See [Commons:File naming](https://commons.wikimedia.org/wiki/Commons:File_naming).
- **Manifest.** `--manifest` writes `.json` or `.csv`, one row per map, sorted by name. The
  first ten columns follow Pattypan's upload spreadsheet: `path`, `name` (without
  extension), `description`, `date`, `source`, `author`, `permission`, `other_versions`,
  `license`, and `categories` (`;`-separated). Then come `file_name`, `sha1`, `region`,
  `data_credit`, `share_alike`, `boundary_year` and `source_release`.
- **Description pages.** `--wikitext` writes `<name>.wikitext` next to each map: an
  [{{Information}}](https://commons.wikimedia.org/wiki/Template:Information) block, the
  licence and the categories. `--description` and `--source` take the same placeholders,
  plus `{data}` for the data credit. By default the description is `{{en|1=Map of {name}}}`
  and the source is "own work, made with map-generator from ⟨data credit⟩".
- **Licence.** The maps' licence is yours to choose (`--license`); nothing is filled in by
  default. Data under a share-alike licence (e.g. CC BY-SA, ODbL) requires a compatible
  share-alike licence for the maps, and `batch` warns when `--license` doesn't look like
  one. The `share_alike` column flags these maps.
- **Dates.** Nothing time-dependent is written unless you pass `--date`. The same input
  therefore always gives the same files, and the same SHA-1s.

## 2. Check what's already there

```sh
scripts/commons-status.py out/manifest.csv
```

This looks each map up by SHA-1 (Commons stores one for every file version) and prints one
status per map:

| Status | Meaning | To do |
| --- | --- | --- |
| `current` | the target file's current version is this map | nothing |
| `elsewhere` | this exact map is the current version of another file | nothing, or rename |
| `older` | this map is an earlier version of the target; Commons has a newer one | check before overwriting |
| `changed` | the target exists with other content | upload as a new version |
| `new` | no such file | upload |

Only `changed` and `new` maps need uploading. The script is read-only, sequential and
uses `maxlag`; `--json` gives machine-readable output.

## 3. Upload

Use an existing tool:

- **[Pattypan](https://commons.wikimedia.org/wiki/Commons:Pattypan).** Open the CSV in
  a spreadsheet and save it as `.xls`, then use *Validate and upload*. The column names
  match Pattypan's default template.
- **[OpenRefine](https://commons.wikimedia.org/wiki/Commons:OpenRefine).** Import the CSV
  and build a Wikimedia Commons schema with `path` as the file path and the wikitext
  columns. OpenRefine can also add structured data.
- **[Pywikibot](https://www.mediawiki.org/wiki/Manual:Pywikibot).** A sketch that uploads
  only what `commons-status.py` reports as `new` or `changed`:

  ```python
  import csv, json, subprocess, time
  import pywikibot

  site = pywikibot.Site("commons", "commons")
  status = {r["file_name"]: r["status"] for r in json.loads(subprocess.check_output(
      ["scripts/commons-status.py", "out/manifest.csv", "--json"]))}
  summary = "Update with 2024 boundaries (map-generator)"  # a new summary for every run
  for row in csv.DictReader(open("out/manifest.csv", encoding="utf-8")):
      if status[row["file_name"]] not in ("new", "changed"):
          continue
      page = pywikibot.FilePage(site, row["file_name"])
      text = open(row["path"].rsplit(".", 1)[0] + ".wikitext", encoding="utf-8").read()
      page.upload(row["path"], comment=summary, text=text,
                  ignore_warnings=["exists"] if page.exists() else False)
      time.sleep(8)
  ```

## 4. Update the pages that show a map

**Same file name.** Every page that shows the map picks up the new version on its own.
Captions and templates with an "as of" date still need editing. Mark the date where it
appears, e.g. `<!-- date -->2024<!-- /date -->`, and replace what's between the markers
with the map's `boundary_year` (or data date) from the manifest. Show a diff before saving:

```python
import re
import pywikibot

site = pywikibot.Site("commons", "commons")
page = pywikibot.Page(site, "Template:COVID-19 Prevalence in US by county")
new = re.sub(r"(<!-- date -->).*?(<!-- /date -->)", r"\g<1>2026-09-23\g<2>", page.text)
pywikibot.showDiff(page.text, new)
if new != page.text and pywikibot.input_yn("Save?", default=False):
    page.text = new
    page.save("Update the data date (map-generator)")
```

**New file name** (e.g. a dated name, so older versions stay available). Pages keep
showing the old file until they are edited:

1. List them on every wiki with [`prop=globalusage`](https://www.mediawiki.org/wiki/API:Globalusage):
   ```sh
   curl -s -A "your-tool (contact)" 'https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2&prop=globalusage&gulimit=max&titles=File:Old_name.svg'
   ```
2. Replace the old name with the new one. Across wikis,
   [CommonsDelinker](https://commons.wikimedia.org/wiki/Commons:CommonsDelinker) does this
   in one request: `{{universal replace|Old name.svg|New name.svg|reason=…}}` on
   [its commands page](https://commons.wikimedia.org/wiki/Commons:CommonsDelinker/commands).
   Administrators and file movers can add commands there; other users can ask one.
   For a few pages on one wiki, Pywikibot's
   [replace.py](https://www.mediawiki.org/wiki/Manual:Pywikibot/replace.py) shows each
   diff and asks before saving (add `-simulate` for a dry run):
   ```sh
   python pwb.py replace -lang:en -family:wikipedia -page:"Some article" "Old name.svg" "New name.svg"
   ```
3. Tag the old file with [{{Superseded|New name.svg}}](https://commons.wikimedia.org/wiki/Template:Superseded),
   and link the versions to each other through `other_versions` in the manifest.

## Etiquette

- **Bot account.** Large or repeated uploads belong on a separate
  [bot account](https://commons.wikimedia.org/wiki/Commons:Bots), with approval at
  [Commons:Bots/Requests](https://commons.wikimedia.org/wiki/Commons:Bots/Requests) for
  sustained runs, and [OAuth](https://www.mediawiki.org/wiki/OAuth/Owner-only_consumers)
  or a bot password rather than your main credentials.
- **Rate.** Upload one file at a time, a few seconds apart (6–10 s works), and send
  [`maxlag=5`](https://www.mediawiki.org/wiki/Manual:Maxlag_parameter) so you back off
  when the servers are busy. Pywikibot does both by default (`put_throttle`, `maxlag`).
- **Don't re-upload unchanged files.** Check with `commons-status.py` first. Byte-identical
  output makes unchanged maps show up as `current`.
- **Overwrite or new name.** Updating a map with newer data under the same name is common
  for maps meant to be kept current, and [Commons:Overwriting existing
  files](https://commons.wikimedia.org/wiki/Commons:Overwriting_existing_files) allows it.
  A map that documents a point in time (e.g. "boundaries as of 2019") should keep its name,
  with the new version uploaded under a new one.
- **Edit summaries.** Use one summary per run that says what changed, and don't reuse the
  previous run's.
