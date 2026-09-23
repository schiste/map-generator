#!/usr/bin/env python3
"""Where each map of a `mapgen batch --manifest` stands on Wikimedia Commons.

Read-only: looks files up by SHA-1 through the MediaWiki API and prints one
line per map:

  current    the target file's current version is this file: nothing to do
  elsewhere  this exact file is the current version of another file name
  older      this file is an earlier version of the target: Commons is newer
  changed    the target exists with other content: upload as a new version
  new        no such file on Commons: upload it

Usage: scripts/commons-status.py manifest.json|manifest.csv [--json]
                                 [--api https://commons.wikimedia.org/w/api.php]

Requests are sequential, identify the tool in their User-Agent and send
maxlag=5, backing off when the servers are busy (see docs/commons.md).
Python 3.8+, standard library only.
"""

import argparse
import csv
import json
import sys
import time
import urllib.parse
import urllib.request

USER_AGENT = "map-generator-commons-status/1.0 (https://github.com/schiste/map-generator)"


def api(url, params):
    params = {**params, "format": "json", "formatversion": "2", "maxlag": "5"}
    request = urllib.request.Request(
        url + "?" + urllib.parse.urlencode(params), headers={"User-Agent": USER_AGENT}
    )
    for attempt in range(6):
        with urllib.request.urlopen(request, timeout=60) as response:
            retry = response.headers.get("Retry-After")
            data = json.load(response)
        if data.get("error", {}).get("code") == "maxlag":
            time.sleep(int(retry or 5) * (attempt + 1))
            continue
        if "error" in data:
            raise RuntimeError(f"API error: {data['error']}")
        return data
    raise RuntimeError("the API stayed lagged; try again later")


def read_manifest(path):
    with open(path, encoding="utf-8") as f:
        if path.endswith(".json"):
            rows = json.load(f)
        else:
            rows = list(csv.DictReader(f))
    return [(r["file_name"], r["sha1"].lower()) for r in rows]


def title(name):
    """A file name as MediaWiki stores it: spaces, first letter capitalised."""
    name = " ".join(name.replace("_", " ").split())
    return name[:1].upper() + name[1:]


def history(url, names):
    """File name -> list of (sha1, timestamp), newest first; missing files absent."""
    found = {}
    for i in range(0, len(names), 50):
        params = {
            "action": "query",
            "prop": "imageinfo",
            "iiprop": "sha1|timestamp",
            "iilimit": "max",
            "titles": "|".join("File:" + n for n in names[i : i + 50]),
        }
        while True:
            data = api(url, params)
            for page in data["query"]["pages"]:
                if page.get("missing") or "imageinfo" not in page:
                    continue
                found.setdefault(page["title"].split(":", 1)[1], []).extend(
                    (v["sha1"], v["timestamp"]) for v in page["imageinfo"]
                )
            if "continue" not in data:
                break
            params = {**params, **data["continue"]}
    return found


def current_names(url, sha1):
    data = api(url, {"action": "query", "list": "allimages", "aisha1": sha1, "ailimit": "max"})
    return [title(i["name"]) for i in data["query"]["allimages"]]


def status(url, rows):
    versions = history(url, sorted({title(name) for name, _ in rows}))
    out = []
    for name, sha1 in rows:
        past = versions.get(title(name), [])
        if past and past[0][0] == sha1:
            out.append(("current", name, ""))
            continue
        if any(s == sha1 for s, _ in past):
            when = next(t for s, t in past if s == sha1)
            out.append(("older", name, f"matches the version of {when}; Commons has {len(past)} versions"))
            continue
        elsewhere = current_names(url, sha1)
        if elsewhere:
            out.append(("elsewhere", name, "current version of " + ", ".join("File:" + e for e in elsewhere)))
        elif past:
            out.append(("changed", name, f"Commons' current version is from {past[0][1]}"))
        else:
            out.append(("new", name, ""))
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("manifest")
    parser.add_argument("--api", default="https://commons.wikimedia.org/w/api.php")
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    args = parser.parse_args()
    results = status(args.api, read_manifest(args.manifest))
    if args.json:
        print(json.dumps([{"status": s, "file_name": n, "detail": d} for s, n, d in results], indent=2))
    else:
        for s, n, d in results:
            print(f"{s}\t{n}\t{d}".rstrip("\t"))
    counts = {}
    for s, _, _ in results:
        counts[s] = counts.get(s, 0) + 1
    print(", ".join(f"{v} {k}" for k, v in sorted(counts.items())), file=sys.stderr)


if __name__ == "__main__":
    main()
