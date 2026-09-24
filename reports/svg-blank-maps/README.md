# Wikimedia Commons SVG blank maps catalog

Catalog source: [Category:SVG blank maps](https://commons.wikimedia.org/wiki/Category:SVG_blank_maps). Metadata batch checkpoints span **2026-09-23 00:18 UTC** to **2026-09-23 17:07 UTC** (UTC); the report was generated after the crawl.

- Recursive category-search branches: **272**
- Unique file pages after deduplication: **34,083**
- Files with at least one usage page: **27,704**
- Total distinct wiki/page usages across these files: **7,001,896**
- Potential SVG/render issue files: **1,513**
- Identical source-file SHA-1 groups: **0**
- Duplicate-review pairs, including filename-based candidates: **177**

## Files

See maps.csv for one row per unique Commons file. Coverage is inferred from filenames, Commons categories, and file descriptions; short descriptions use Commons prose where it is useful and generated fallbacks where it is absent or generic. Some labels may be ambiguous, so each row links to the Commons file page and its categories for review.

## Most-used files

| Map | Distinct usage pages |
|---|---:|
| [USA location map.svg](<https://commons.wikimedia.org/wiki/File:USA_location_map.svg>) | 766,809 |
| [France location map-Regions and departements-2016.svg](<https://commons.wikimedia.org/wiki/File:France_location_map-Regions_and_departements-2016.svg>) | 734,390 |
| [Mexico location map.svg](<https://commons.wikimedia.org/wiki/File:Mexico_location_map.svg>) | 514,646 |
| [Poland adm location map.svg](<https://commons.wikimedia.org/wiki/File:Poland_adm_location_map.svg>) | 453,050 |
| [Canada location map.svg](<https://commons.wikimedia.org/wiki/File:Canada_location_map.svg>) | 260,524 |
| [Germany location map.svg](<https://commons.wikimedia.org/wiki/File:Germany_location_map.svg>) | 242,256 |
| [Mexico States blank map.svg](<https://commons.wikimedia.org/wiki/File:Mexico_States_blank_map.svg>) | 205,119 |
| [Blank map.svg](<https://commons.wikimedia.org/wiki/File:Blank_map.svg>) | 150,797 |
| [France location map.svg](<https://commons.wikimedia.org/wiki/File:France_location_map.svg>) | 146,420 |
| [Russia 2014 edcp location map.svg](<https://commons.wikimedia.org/wiki/File:Russia_2014_edcp_location_map.svg>) | 135,732 |

## Usage counts

usage_count_wikimedia_pages counts distinct `(wiki, page ID)` pairs returned by the [MediaWiki GlobalUsage API](https://www.mediawiki.org/wiki/API:Globalusage) for that file. It counts pages that use the file across Wikimedia projects; repeated embeds on one page count once. Collection spans multiple API requests rather than one atomic snapshot, so values may change during the crawl and after it finishes.

## Duplicate review

duplicate-candidates.csv lists exact SHA-1 matches and filename/aspect-ratio candidates. Exact matches have identical source bytes. Filename-based matches are leads for visual review, not confirmed duplicates; files can differ by projection, borders, or administrative detail.

## Possible rendering issues

possible-render-issues.csv includes files tagged by Commons as invalid SVG or as having broken file links, plus files with missing image metadata, missing SVG preview URLs, or failed thumbnail requests. Commons categories flag 1,360 files as invalid SVG and 146 as having broken file links; two files are in both categories, for 1,504 distinct category-flagged files. These are review signals, not proof a map is unusable. The thumbnail pass reached 34,074 previews; nine requests returned HTTP 429 and remain unconfirmed, so they are marked as rate-limited rather than broken. A reachable preview does not validate geographic content or visual accuracy.

Recursive deep-category searches were partitioned across the country branch to stay within MediaWiki limits of 256 categories and five levels. Coverage favors specific administrative categories and adds the named place and its parent region for locator titles such as ‘place in region’. It falls back to the file title or description when category detail is absent. `Blank map.svg` is labeled as a generic locator template because its SVG has no fixed geographic region or country boundaries. Coverage is a category/name signal, not a geospatial validation of each SVG.

## Recursive-search warnings

- Category:SVG locator maps of Armenia: deepcat SPARQL failed; recursive categorymembers fallback covered 1 categories
- Category:SVG locator maps of the Czech Republic: deepcat SPARQL failed; recursive categorymembers fallback covered 5 categories
- Category:SVG locator maps of Tuvalu: deepcat SPARQL failed; recursive categorymembers fallback covered 3 categories
