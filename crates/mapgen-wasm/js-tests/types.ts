// Compile-time test of the TypeScript definitions: `npm run typecheck`.
// Lines marked @ts-expect-error must fail to type-check, or tsc reports an error.
import { MapGenerator, themes, bboxPresets, version, reshape, parseRecipe, recipeToCsv, type Recipe, type Country, type MapOutput, type RenderSpec, type MatchOutput, type ReshapeOutput } from "../pkg/node/mapgen_wasm.js";

const gen = new MapGenerator();
const n: number = gen.setSubject("{}");
gen.setSubject("{}", { dataset: "ne-admin1", filterProperty: "adm0_a3" });
gen.setContext();
gen.setContext("{}");
gen.setLakes("{}", { dataset: "ne-lakes" });
gen.setDisputed("{}", { dataset: "ne-disputed" });
gen.setDisputed();
gen.setDisputedAreas("{}", { dataset: "ne-disputed-areas" });
gen.setUnits("map_id,data_unit_id\nA,U\n");
gen.setUnits("region|unit\nA|U\n", { mapColumn: "region", unitColumn: "unit" });
gen.setUnits();
const splitUnits: [string, number][] | undefined = gen.render({ dissolve: true }).units?.splitUnits;
gen.setSubject("{}", { dataset: "ne-admin0", worldview: "IND" });
gen.setSubject("{}", { dataset: "geoboundaries", parentProperty: "parent" });
gen.setSubject("{}", { parentNameProperty: "state", countryProperty: "iso3" });
gen.setSubject("{}", { dataset: "ne-admin1", languages: ["fr", "zh-Hant"], nameLanguageProperty: "name_{lang}" });
gen.render({ labels: true, languages: ["fr", "zh-Hant"] });

const out: MapOutput = gen.render();
const svg: string = out.svg;
const proj: "laea" | "equal-earth" | "albers" | "lcc" = out.projection;
const spec: RenderSpec = {
  region: "FRA",
  theme: "dark",
  colors: { water: "#123456", earth: "tan", contextLand: "#eee" },
  frame: "world",
  projection: "equal-earth",
  format: "html",
};
const conic: RenderSpec = {
  projection: "albers",
  parallels: [29.5, 45.5],
  colors: { outline: "#333", disputedBorder: "red" },
  outlineWidth: 1.2,
  parentBorderWidth: 1.5,
  leaders: false,
  borderMode: "regions",
  boundaryYear: "2018",
  sourceRelease: "USA-ADM2-52423323",
  curvedLabels: true,
  labelMinScale: 0.8,
  snap: 0,
  insets: "none",
  maxInsets: 3,
};
gen.render(conic);
const insetIds: string[] | undefined = out.insets[0]?.ids;
gen.render(spec);
const regions: string[] = gen.regions();
const water: string | undefined = themes()["wikimedia"]?.["water"];
const europe = bboxPresets()["europe"];
const v: string = version();

// @ts-expect-error misspelled option
gen.render({ colours: {} });
// @ts-expect-error unknown theme
gen.render({ theme: "neon" });
// @ts-expect-error unknown colour slot
gen.render({ colors: { sea: "blue" } });
// @ts-expect-error unknown dataset
gen.setSubject("{}", { dataset: "gadm" });
// @ts-expect-error width must be a number
gen.render({ width: "wide" });
// @ts-expect-error unknown projection
gen.render({ projection: "mercator" });
// @ts-expect-error insets is "auto" | "none"
gen.render({ insets: true });

export { n, svg, proj, regions, water, europe, v, insetIds, splitUnits };

const m: MatchOutput = gen.matchCodes({ codes: ["FR-75"], codePrefix: "" });
const missing: string[] = m.dataNotOnMap;
const r: ReshapeOutput = reshape({ table: "a,b\n", codeColumn: "a", crosswalk: { table: "from,to\n" } });
const csvOut: string | undefined = r.csv;
void missing; void csvOut;

const recipe: Recipe = parseRecipe("key,value\ndataset,countries\nregion,FRA\n");
const recipeText: string = recipeToCsv({ dataset: recipe.dataset, regions: ["FRA"], spec: recipe.spec });
const picked: Country[] = gen.countries();
const resolved: string[] = gen.resolveRegions(["France"]).codes;
gen.render({ regions: resolved });
void recipeText; void picked;
