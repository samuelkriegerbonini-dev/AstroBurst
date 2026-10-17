import type {
  MastColumn,
  MastImageEntry,
  MastImageSet,
  MastMission,
  MastMissionId,
  MastProduct,
  MastProductClass,
  MastProductList,
  MastRow,
  MastSearchPage,
} from "../../../shared/types/mast";

const CHECKED_ON = "2026-10-16";

function mission(
  id: MastMissionId,
  label: string,
  availability: MastMission["availability"],
  caps: MastMission["capabilities"],
  quick: MastMission["quickFilters"] = [],
): MastMission {
  return {
    id,
    label,
    availability,
    capabilities: caps,
    views: { gallery: caps.images, table: true },
    quickFilters: quick,
    defaultColumns: [],
    note: null,
    checkedOn: CHECKED_ON,
  };
}

export const MISSIONS: MastMission[] = [
  mission("jwst", "JWST", "open", { search: true, images: true, products: true }, [
    { id: "level", label: "Level", column: "productLevel", options: ["combined", "all"] },
    { id: "instrument", label: "Instrument", column: "instrume", options: ["NIRCAM", "MIRI", "NIRSPEC", "NIRISS", "FGS"] },
    { id: "filter", label: "Filter", column: "opticalElements", options: null },
    { id: "date", label: "Date", column: "date_obs", options: null },
    { id: "public", label: "Public only", column: "access", options: null },
  ]),
  mission("hst", "HST", "open", { search: true, images: true, products: true }, [
    { id: "instrument", label: "Instrument", column: "sci_instrume", options: ["WFC3", "ACS", "WFPC2", "NICMOS", "STIS", "COS"] },
    { id: "filter", label: "Filter", column: "sci_spec_1234", options: null },
    { id: "date", label: "Date", column: "sci_start_time", options: null },
  ]),
  mission("roman", "Roman", "token", { search: true, images: true, products: true }),
  mission("iue", "IUE", "open", { search: true, images: false, products: false }),
  mission("classy", "CLASSY", "open", { search: true, images: false, products: false }),
  mission("ullyses", "ULLYSES", "open", { search: true, images: false, products: false }),
  mission("roman_cgi", "Roman CGI", "token", { search: false, images: false, products: false }),
  mission("roman_spectra", "Roman Spectra", "not_deployed", { search: false, images: false, products: false }),
  mission("tess", "TESS", "not_deployed", { search: false, images: false, products: false }),
  mission("multi_mission", "All missions", "not_deployed", { search: false, images: false, products: false }),
];

export function missionById(id: MastMissionId): MastMission {
  const m = MISSIONS.find((x) => x.id === id);
  if (!m) throw new Error(id);
  return m;
}

export const COLUMNS: MastColumn[] = [
  { name: "fileSetName", label: "Dataset", kind: "text", sortable: true, unit: null },
  { name: "productLevel", label: "Lvl", kind: "text", sortable: true, unit: null },
  { name: "effexptm", label: "Exp (s)", kind: "float", sortable: true, unit: "s" },
  { name: "date_obs", label: "Date obs (UTC)", kind: "datetime", sortable: true, unit: null },
  { name: "program", label: "Program", kind: "int", sortable: true, unit: null },
  { name: "_prog_obs", label: "Prog.Obs", kind: "text", sortable: false, unit: null },
];

export function row(datasetId: string, over: Partial<MastRow> = {}): MastRow {
  return {
    datasetId,
    levelClass: "combined",
    level: "3",
    instrument: "NIRCAM",
    filter: "F187N",
    filters: ["F090W", "F187N", "F200W", "F335M", "F444W", "F444W-F470N"],
    expType: "NRC_IMAGE",
    target: "M 16",
    program: "2739",
    observation: "1",
    dateObs: "2022-08-14T08:33:45.2960000",
    expTimeS: 5797.86,
    sepArcmin: null,
    sepVerified: false,
    access: "PUBLIC",
    releaseDate: "2022-08-14T00:00:00",
    raDeg: 274.688,
    decDeg: -13.792,
    sRegion: "POLYGON ICRS 274.69 -13.89 274.76 -13.91 274.77 -13.79",
    values: { fileSetName: datasetId, productLevel: "3", effexptm: 5797.86, date_obs: "2022-08-14T08:33:45.2960000", program: 2739, _prog_obs: "2739.1" },
    ...over,
  };
}

export function page(rows: MastRow[], over: Partial<MastSearchPage> = {}): MastSearchPage {
  return {
    requestId: "g1:search",
    mission: "jwst",
    columns: COLUMNS,
    rows,
    offset: 0,
    limit: 200,
    hasMore: false,
    totalReported: rows.length,
    totalReliable: true,
    radiusArcmin: 3,
    radiusClamped: false,
    searchPosition: { raDeg: 274.688, decDeg: -13.792 },
    applied: { conditions: [{ productLevel: "3" }], sortBy: [], sortDesc: [] },
    droppedConditions: [],
    requestBody: { target: ["274.688 -13.792"], radius: 3, conditions: [{ productLevel: "3" }] },
    messages: [],
    unresolvedTargets: [],
    elapsedMs: 1700,
    fromCache: false,
    ...over,
  };
}

function entry(key: string, group: string, filter: string | null, over: Partial<MastImageEntry> = {}): MastImageEntry {
  return {
    key,
    label: filter ?? key,
    group,
    productFilename: `${key}.fits`,
    thumbnailFilename: `${key}_thumb.jpg`,
    previewFilename: `${key}.jpg`,
    level: "3",
    suffix: group,
    detector: null,
    filter,
    scienceGroup: null,
    colour: false,
    mastUri: `mast:JWST/product/${key}.fits`,
    ...over,
  };
}

export const L3_FILTERS = ["F090W", "F187N", "F200W", "F335M", "F444W", "F444W-F470N"];

export function jwstL3Set(datasetId = "jw02739-o001_t001_nircam"): MastImageSet {
  const entries = L3_FILTERS.map((f) => entry(`${datasetId}_clear-${f.toLowerCase()}_i2d`, "i2d", f));
  return {
    mission: "jwst",
    datasetId,
    entries,
    groups: [{ key: "i2d", label: "i2d", count: entries.length }],
    defaultGroup: "i2d",
    representative: 1,
    elapsedMs: 800,
    fromCache: false,
  };
}

export function hstSet(datasetId = "ICK909030"): MastImageSet {
  const lower = datasetId.toLowerCase();
  const entries = [
    entry(`${lower}_drc`, "combined", "F502N"),
    entry(`${lower}_drz`, "combined", "F502N"),
    entry(`${lower.slice(0, 6)}abq_flc`, "calibrated", "F502N"),
    entry(`${lower.slice(0, 6)}abq_raw`, "raw", "F502N"),
    entry(`hst_15238_20_wfc3_ir_total_${lower.slice(0, 6)}_drz_color`, "hap", null, { colour: true, scienceGroup: "HapScience" }),
  ];
  return {
    mission: "hst",
    datasetId,
    entries,
    groups: [
      { key: "combined", label: "Combined", count: 2 },
      { key: "calibrated", label: "Calibrated", count: 1 },
      { key: "raw", label: "Raw", count: 1 },
      { key: "hap", label: "HAP aligned", count: 1 },
    ],
    defaultGroup: "combined",
    representative: 0,
    elapsedMs: 600,
    fromCache: false,
  };
}

export function product(filename: string, cls: MastProductClass, sizeBytes: number, over: Partial<MastProduct> = {}): MastProduct {
  const opens: MastProduct["opensAs"] = cls === "image" || cls === "cube" || cls === "spectrum" || cls === "ramp" || cls === "catalog" ? cls : null;
  return {
    datasetId: "jw02739-o001_t001_nircam",
    filename,
    sizeBytes,
    category: cls === "reference" ? "REFERENCE" : cls === "guidestar" ? "GUIDESTAR" : "SCIENCE",
    type: "science",
    suffix: "i2d",
    cls,
    opensAs: opens,
    access: "PUBLIC",
    mastUri: `mast:JWST/product/${filename}`,
    downloadUrl: `https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:JWST/product/${filename}`,
    badges: [],
    ...over,
  };
}

export function productList(): MastProductList {
  const products: MastProduct[] = [
    product("a_i2d.fits", "image", 5_420_000_000),
    product("b_i2d.fits", "image", 5_420_000_000),
    product("a_cat.ecsv", "catalog", 990_000_000),
    product("a_thumb.jpg", "preview", 13_500_000),
    product("a_asn.json", "association", 10_000),
    product("x_jit.fits", "auxiliary", 5_000),
    product("y_trl.fits", "other", 2_000),
    product("crds_1.fits", "reference", 6_000_000_000),
    product("crds_2.fits", "reference", 4_000_000_000),
    product("gs_1.fits", "guidestar", 1_770_000_000),
  ];
  const classes: MastProductClass[] = ["image", "catalog", "preview", "association", "auxiliary", "other", "reference", "guidestar"];
  const groups = classes.map((cls) => {
    const of = products.filter((p) => p.cls === cls);
    return { cls, count: of.length, bytes: of.reduce((s, p) => s + p.sizeBytes, 0), defaultVisible: cls === "image" || cls === "catalog" };
  });
  return {
    mission: "jwst",
    datasetIds: ["jw02739-o001_t001_nircam"],
    products,
    groups,
    totalCount: products.length,
    totalBytes: products.reduce((s, p) => s + p.sizeBytes, 0),
    elapsedMs: 29_500,
    fromCache: false,
    cachedUntil: "2026-10-17T04:10:00Z",
  };
}
