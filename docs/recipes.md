# Map recipes

A recipe is a whole map in a small CSV file: which regions to draw and every design setting.
You can edit it in a spreadsheet, import and export it in the
[playground](https://map-generator.toolforge.org/), and render it with the API:

```sh
curl -X POST https://map-generator.toolforge.org/api/v1/render \
  -H 'Content-Type: text/csv' --data-binary @benelux.csv -o benelux.svg
```

## Format

Two columns, `key` and `value`:

```csv
# Lines starting with # are comments.
key,value
dataset,countries
region,Belgium
region,NLD
region,LU
title,Benelux
width,1200
labels,true
languages,fr;nl;de
theme,light
color-water,#c6ecff
color-land,#fdfdf5
projection,laea
```

- **`dataset`**: `countries` (Natural Earth countries, `ne-admin0`), `subdivisions` (states,
  provinces, départements of the chosen countries, `ne-admin1`), or any dataset id from
  [`/api/v1/datasets`](https://map-generator.toolforge.org/api/v1/datasets), e.g. `us-counties` or
  `geoboundaries-adm1`.
- **`region`**, one row per region: a code (`FRA`), a name (`France`, or a name in another
  language), an ISO 3166-1 alpha-2 code (`fr`) or a Wikidata item (`Q142`), as the
  Choropleth map template takes them. You can also write them in one row:
  `regions,France;DEU;Italy`. For `subdivisions`, the regions are the countries whose
  subdivisions are drawn.
- **`worldview`**: a Natural Earth point of view for disputed borders, e.g. `IND` (API only).
- **Every other key** is a map setting, spelled exactly as the API's query parameters
  ([api.md](api.md#map-parameters)). Lists use `;` (a comma would split the CSV cell).

| Key | Values |
| --- | --- |
| `title` | text |
| `width`, `padding` | pixels |
| `theme` | `wikimedia`, `light`, `dark`, `mono` |
| `color-<slot>` | any CSS colour. Slots: `background`, `water`, `land`, `context-land`, `border`, `outline`, `coast`, `context-border`, `lake-border`, `disputed-border`, `label` |
| `labels`, `credit`, `css-vars`, `leaders`, `curved-labels` | `true` / `false` |
| `languages` | BCP 47 tags, e.g. `fr;ar;zh-Hans` |
| `target` | `commons` (curved labels as rotated letters; for Wikimedia Commons) or `web` |
| `projection` | `auto`, `laea`, `equal-earth`, `albers`, `lcc` (and `parallels`, e.g. `29.5;45.5`) |
| `frame` | `auto`, `all`, `world`; or `bbox`: a preset (`europe`…) or `west;south;east;north` |
| `insets`, `max-insets` | `auto` / `none`; a number |
| `border-mode` | `layer` or `regions` |
| `border-width`, `parent-border-width`, `outline-width`, `context-border-width`, `disputed-border-width`, `label-size`, `label-min-scale`, `simplify`, `min-area`, `margin`, `snap`, `center-lon` | numbers |

Unknown keys are errors, not silently ignored. So a typo like `colour-water` is reported
with the recipe's own spelling.

A table with a `country`, `region`, `code`, `iso2` or `iso3` column (such as a Maphue export)
is also accepted. It is read as a list of regions with default settings.

## Where recipes come from

- The playground's **Download recipe** button writes the current map as a recipe.
- A recipe's keys are the API's parameters, so every recipe can also be a map URL. The
  recipe above is
  `/api/v1/maps/ne-admin0/BEL,LUX,NLD.svg?title=Benelux&width=1200&labels=true&languages=fr,nl,de&theme=light&color-water=%23c6ecff&color-land=%23fdfdf5&projection=laea`.
