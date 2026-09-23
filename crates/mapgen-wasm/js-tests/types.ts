// Compile-time test of the TypeScript definitions: `npm run typecheck`.
// Lines marked @ts-expect-error must fail to type-check, or tsc reports an error.
import { MapGenerator, themes, bboxPresets, version, type MapOutput, type RenderSpec } from "../pkg/node/mapgen_wasm.js";

const gen = new MapGenerator();
const n: number = gen.setSubject("{}");
gen.setSubject("{}", { dataset: "ne-admin1", filterProperty: "adm0_a3" });
gen.setContext();
gen.setContext("{}");
gen.setLakes("{}", { dataset: "ne-lakes" });

const out: MapOutput = gen.render();
const svg: string = out.svg;
const proj: "laea" | "equal-earth" = out.projection;
const spec: RenderSpec = {
  region: "FRA",
  theme: "dark",
  colors: { water: "#123456", earth: "tan", contextLand: "#eee" },
  frame: "world",
  projection: "equal-earth",
  format: "html",
};
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

export { n, svg, proj, regions, water, europe, v };
