# SVG contract, version 1

What tools that read or colour map-generator's maps (such as
[Maphue](https://github.com/schiste/map-coloring)) can rely on. Every map declares the
version it follows on its root element, `data-mapgen-contract="1"`. The API also reports it
in the `X-Mapgen-Contract` header and at `/api/v1/version`.

`crates/mapgen-data/tests/contract.rs` checks these rules on the golden fixture and every
gallery map.

## Guaranteed

### Root
- `<svg>` has `width`, `height` and `viewBox="0 0 width height"` (pixels), and
  `data-mapgen-contract`.
- `data-boundary-year` and `data-source-release` are present when the boundary version is
  known.
- `<title>` holds the map title, when given. `<desc id="description">` holds a description
  for screen readers (`alt`), and `<desc id="attribution">` the data credit, when there are
  any.
- With a visible title (`show-title`), a 40 px band above the map holds
  `text#map-title.mg-title`, and the `viewBox` starts at `y = -40`. The map keeps its
  coordinates: `0,0` is always its top-left corner. With a `caption`, `text#caption.mg-caption`
  sits in a band under the map. Neither is inside a layer group.

### Layers
Main-map groups, bottom to top, by `id` (a group is absent when empty):

| id | Content |
| --- | --- |
| `background` | a `rect` |
| `water` | a `path` |
| `context` | neighbouring countries |
| `context-borders` | their borders |
| `land` | the regions being mapped |
| `lakes` | lakes |
| `disputed-areas` | hatched disputed areas |
| `borders` | borders, one path per kind |
| `labels` | labels |

Insets are `g.mg-inset` groups (ids `inset-1`, `inset-2`…) after the main layers. They
contain the same layers as classes (`g.mg-land`, `g.mg-labels`…), plus a `rect.mg-inset-frame`.

### Regions
Every region of the mapped area is exactly one `path` with the class `mg-land`, in `#land`
or an inset's `.mg-land`, with:

| Attribute | Content |
| --- | --- |
| `id` | unique, valid XML id derived from the code |
| `data-code` | the region's code as in the data: ISO 3166-2 (`FR-75`), FIPS (`US-31109`), ADM0_A3 (`FRA`)… |
| `data-name` | its name |
| `data-parent` | code of the enclosing unit (`FR-IDF`, `US-31`), when known |
| `data-parent-name` | its name, when known (only together with `data-parent`) |
| `data-unit` | space-separated data units the region belongs to, when set |
| `data-wikidata` | the region's Wikidata item (`Q142`), when known (Natural Earth layers) |
| `class` | `mg-land`, the feature class (`subdivision`, `country`, `region`…), and the lowercase ISO 3166-1 alpha-2 code of its country (`fr`) when known |

Each region has a `<title>` child for tooltips ("Lancaster, Nebraska").

Neighbouring countries are `path.mg-context` in `#context` (or `.mg-context`), with the same
attributes.

### Colours
- Colours are defined only in the `<style>` block, through `.mg-*` classes. Region and
  neighbour paths have no `fill` or `style` attribute.
- So a consumer can colour regions with its own CSS (`[data-code="FR-75"]{fill:#c00}`,
  `[class~="fr"]{…}`) or inline styles, and they override the map's colours.
- With `css-vars`, every colour is `var(--mg-<slot>, <fallback>)`. The slots are listed at
  `/api/v1/themes` and by `mapgen themes`.

### Labels
- Labels are only in `#labels` (or an inset's `.mg-labels`), with the class `mg-label`.
- Multilingual labels are `<switch>` elements whose children carry `systemLanguage`, with
  the default name last.
- Curved labels are rotated letters in `g.mg-label[aria-label]` (the `commons` target) or
  `textPath` (the `web` target).

## Not guaranteed

These can change in any release, without a contract version bump:
- path data (simplification, precision, coordinates), and the order of elements within a
  layer;
- label positions, sizes and which labels fit;
- theme colours and stroke widths (use CSS or `css-vars` to set your own);
- element ids other than layers, insets and regions.

## Versioning

- Adding an attribute, class or layer keeps the version.
- Removing one, or changing what it means, increments the version and is noted in the
  changelog. The API keeps serving the previous version under its `/api/v1` path until that
  path's announced sunset.
