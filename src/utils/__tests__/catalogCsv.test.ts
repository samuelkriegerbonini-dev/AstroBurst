import { describe, it, expect } from "vitest";
import {
  CSV_LINE_END,
  buildCsv,
  catalogCsvFileName,
  catalogRowsCsv,
  csvCell,
  csvEscape,
  matchesCsv,
  sourcesCsv,
  type CsvColumn,
} from "../catalogCsv";
import type { CrossMatchEntry, MeasuredSource, PlacedCatalogRow } from "../../shared/types/catalog";

function source(overrides: Partial<MeasuredSource> = {}): MeasuredSource {
  return {
    x: 30,
    y: 31.5,
    ra: 150.000123,
    dec: 2.000456,
    flux: 1234.5,
    fwhm: 4.7,
    snr: 120,
    mag_inst: -7.73,
    mag_ab: 17.27,
    saturated: false,
    ...overrides,
  };
}

function placedRow(overrides: Partial<PlacedCatalogRow> = {}): PlacedCatalogRow {
  return {
    id: "3403818172572314624",
    ra: 150.1,
    dec: 2.1,
    ra_epoch: 150.1001,
    dec_epoch: 2.1001,
    pm_ra_masyr: 1.2,
    pm_dec_masyr: -3.4,
    g: 8.5,
    bp: 8.9,
    rp: 7.95,
    bp_rp: 0.95,
    parallax_mas: 2.5,
    x: 10.25,
    y: 20.5,
    on_image: true,
    ...overrides,
  };
}

describe("csvEscape", () => {
  it("leaves plain text alone and quotes separators, quotes and line breaks per RFC 4180", () => {
    expect(csvEscape("plain")).toBe("plain");
    expect(csvEscape("")).toBe("");
    expect(csvEscape("a,b")).toBe('"a,b"');
    expect(csvEscape('say "hi"')).toBe('"say ""hi"""');
    expect(csvEscape("two\nlines")).toBe('"two\nlines"');
    expect(csvEscape("cr\rhere")).toBe('"cr\rhere"');
  });
});

describe("csvCell", () => {
  it("writes empty cells for missing or non-finite values and plain text for the rest", () => {
    expect(csvCell(null)).toBe("");
    expect(csvCell(undefined)).toBe("");
    expect(csvCell(Number.NaN)).toBe("");
    expect(csvCell(Number.POSITIVE_INFINITY)).toBe("");
    expect(csvCell(0)).toBe("0");
    expect(csvCell(-2.5)).toBe("-2.5");
    expect(csvCell(true)).toBe("true");
    expect(csvCell(false)).toBe("false");
    expect(csvCell("x,y")).toBe('"x,y"');
  });
});

describe("buildCsv", () => {
  it("emits a header line, one CRLF-terminated line per item and escapes every cell", () => {
    const columns: CsvColumn<{ name: string; value: number | null }>[] = [
      { header: "name", value: (item) => item.name },
      { header: "odd,header", value: (item) => item.value },
    ];
    const csv = buildCsv(columns, [
      { name: "alpha", value: 1 },
      { name: "be,ta", value: null },
    ]);
    expect(csv).toBe(`name,"odd,header"${CSV_LINE_END}alpha,1${CSV_LINE_END}"be,ta",${CSV_LINE_END}`);
    expect(buildCsv(columns, [])).toBe(`name,"odd,header"${CSV_LINE_END}`);
  });
});

describe("catalog exports", () => {
  it("writes the catalog columns in the documented order", () => {
    const lines = catalogRowsCsv([placedRow(), placedRow({ id: "no,pm", pm_ra_masyr: null, pm_dec_masyr: null, x: null, y: null, on_image: false })]).split(CSV_LINE_END);
    expect(lines[0]).toBe("id,ra,dec,ra_epoch,dec_epoch,pm_ra_masyr,pm_dec_masyr,parallax_mas,g,bp,rp,bp_rp,x,y,on_image");
    expect(lines[1]).toBe("3403818172572314624,150.1,2.1,150.1001,2.1001,1.2,-3.4,2.5,8.5,8.9,7.95,0.95,10.25,20.5,true");
    expect(lines[2]).toBe('"no,pm",150.1,2.1,150.1001,2.1001,,,2.5,8.5,8.9,7.95,0.95,,,false');
    expect(lines[3]).toBe("");
    expect(lines).toHaveLength(4);
  });

  it("writes every source with an empty mag_ab when the image is not calibrated", () => {
    const lines = sourcesCsv([source(), source({ mag_ab: null, saturated: true })]).split(CSV_LINE_END);
    expect(lines[0]).toBe("x,y,ra,dec,flux,mag_inst,mag_ab,fwhm,snr,saturated");
    expect(lines[1]).toBe("30,31.5,150.000123,2.000456,1234.5,-7.73,17.27,4.7,120,false");
    expect(lines[2]).toBe("30,31.5,150.000123,2.000456,1234.5,-7.73,,4.7,120,true");
  });

  it("flattens star and catalog row fields of a match into one line", () => {
    const entry: CrossMatchEntry = {
      star: source({ mag_ab: null }),
      row: { id: "gaia1", ra: 150.0001, dec: 2.0001, g: 20, bp: null, rp: null, bp_rp: 0.8 },
      sep_arcsec: 0.3,
      d_ra_arcsec: 0.2,
      d_dec_arcsec: -0.1,
    };
    const lines = matchesCsv([entry]).split(CSV_LINE_END);
    expect(lines[0]).toBe(
      "id,x,y,ra,dec,flux,mag_inst,mag_ab,fwhm,snr,cat_ra,cat_dec,g,bp,rp,bp_rp,sep_arcsec,d_ra_arcsec,d_dec_arcsec",
    );
    expect(lines[1]).toBe("gaia1,30,31.5,150.000123,2.000456,1234.5,-7.73,,4.7,120,150.0001,2.0001,20,,,0.8,0.3,0.2,-0.1");
  });
});

describe("catalogCsvFileName", () => {
  it("derives the file name from the image stem and the export kind", () => {
    expect(catalogCsvFileName("C:\\data\\m42_stack.fits", "matches")).toBe("m42_stack_matches.csv");
    expect(catalogCsvFileName("/home/obs/ngc7000.fit#hdu=1", "catalog")).toBe("ngc7000_catalog.csv");
    expect(catalogCsvFileName("jw01234.fits#array=roman.dq", "sources")).toBe("jw01234_sources.csv");
    expect(catalogCsvFileName(null, "sources")).toBe("catalog_sources.csv");
    expect(catalogCsvFileName("", "catalog")).toBe("catalog_catalog.csv");
  });
});
