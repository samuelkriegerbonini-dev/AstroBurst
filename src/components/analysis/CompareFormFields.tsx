import { memo, type ReactNode } from "react";
import type { CatalogueFluxUnit, CompareColumn, CompareCutOp } from "../../shared/types/catalogCompare";
import {
  CATALOGUE_FLUX_UNITS,
  COMPARE_CUT_OPS,
  FLUX_UNIT_LABELS,
  type CompareCutDraft,
  type CompareErrorKind,
  type CompareForm,
  type CompareRadiusUnit,
} from "../../utils/catalogCompare";
import { DecimalField, Toggle } from "../ui";
import { columnOptions } from "./compareFormFieldsModel";

export interface CompareFormFieldsProps {
  form: CompareForm;
  columns: readonly CompareColumn[];
  disabled?: boolean;
  onChange: (next: CompareForm) => void;
}

type ColumnMember = "raColumn" | "decColumn" | "fluxColumn" | "errorColumn" | "idColumn" | "classColumn";

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-cyan-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const CUT_BUTTON_CLASS =
  "px-2 py-0.5 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";
const BLOCK_CLASS = "grid grid-cols-2 gap-2";

const ERROR_KIND_OPTIONS: readonly (readonly [CompareErrorKind, string])[] = [
  ["none", "none"],
  ["flux_err", "error column"],
  ["flux_ivar", "inverse variance column"],
];
const RADIUS_UNITS: readonly CompareRadiusUnit[] = ["arcsec", "px"];
const NEW_CUT: CompareCutDraft = { column: "", op: "==", value: "" };

const ZERO_POINT_TITLE = "Zero point of the catalogue flux in AB magnitudes: m = zero point - 2.5 log10(flux)";
const CLASS_TITLE = "Shown as the catalogue wrote it; a catalogue class is not a resolution verdict";
const MISSING_VALUE_TITLE = "A cell that holds this number counts as missing, for example -999";
const BAND_TITLE = "Band of the catalogue column; a warning is given when the filter of the image differs";
const SNR_TITLE = "Catalogue flux over catalogue error; needs an error column";
const NEIGHBOUR_TITLE = "Leave out a row that has another table row closer than this";

function captioned(caption: string, control: ReactNode): ReactNode {
  return (
    <label className="flex flex-col gap-0.5">
      <span className="text-[9px] text-zinc-500 uppercase">{caption}</span>
      {control}
    </label>
  );
}

function nameOptions(names: readonly string[]): ReactNode {
  return names.map((name) => (
    <option key={name} value={name}>
      {name}
    </option>
  ));
}

function CompareFormFields({ form, columns, disabled = false, onChange }: CompareFormFieldsProps) {
  const set = <K extends keyof CompareForm>(member: K, value: CompareForm[K]) => onChange({ ...form, [member]: value });
  const setCut = (index: number, cut: CompareCutDraft) =>
    onChange({ ...form, cuts: form.cuts.map((item, i) => (i === index ? cut : item)) });

  const columnSelect = (member: ColumnMember, emptyText: string, numericOnly: boolean, title?: string) => (
    <select
      data-field={member}
      value={form[member]}
      disabled={disabled}
      title={title}
      onChange={(e) => set(member, e.target.value)}
      className={SELECT_CLASS}
    >
      <option value="">{emptyText}</option>
      {nameOptions(columnOptions(columns, form[member], numericOnly))}
    </select>
  );

  return (
    <div className="space-y-2" data-compare-form>
      <div className={BLOCK_CLASS}>
        {captioned("RA column", columnSelect("raColumn", "choose", true))}
        {captioned("Dec column", columnSelect("decColumn", "choose", true))}
        {captioned("Flux column", columnSelect("fluxColumn", "choose", true))}
        {captioned(
          "Flux unit",
          <select
            data-field="fluxUnit"
            value={form.fluxUnit}
            disabled={disabled}
            onChange={(e) => set("fluxUnit", e.target.value as CatalogueFluxUnit)}
            className={SELECT_CLASS}
          >
            {CATALOGUE_FLUX_UNITS.map((unit) => (
              <option key={unit} value={unit}>
                {FLUX_UNIT_LABELS[unit]}
              </option>
            ))}
          </select>,
        )}
        {form.fluxUnit === "zero_point" &&
          captioned(
            "Zero point",
            <DecimalField
              data-field="zeroPointText"
              value={form.zeroPointText}
              disabled={disabled}
              title={ZERO_POINT_TITLE}
              onChange={(text) => set("zeroPointText", text)}
              className={INPUT_CLASS}
            />,
          )}
        {captioned(
          "Catalogue error",
          <select
            data-field="errorKind"
            value={form.errorKind}
            disabled={disabled}
            onChange={(e) => set("errorKind", e.target.value as CompareErrorKind)}
            className={SELECT_CLASS}
          >
            {ERROR_KIND_OPTIONS.map(([kind, text]) => (
              <option key={kind} value={kind}>
                {text}
              </option>
            ))}
          </select>,
        )}
        {form.errorKind !== "none" && captioned("Error column", columnSelect("errorColumn", "choose", true))}
        {captioned("Id column", columnSelect("idColumn", "row number", false))}
        {captioned("Class column", columnSelect("classColumn", "none", false, CLASS_TITLE))}
        {captioned(
          "Missing value",
          <DecimalField
            data-field="missingValueText"
            optional
            value={form.missingValueText}
            placeholder="none"
            disabled={disabled}
            title={MISSING_VALUE_TITLE}
            onChange={(text) => set("missingValueText", text)}
            className={INPUT_CLASS}
          />,
        )}
        {captioned(
          "Catalogue band",
          <input
            type="text"
            data-field="catalogueBand"
            value={form.catalogueBand}
            placeholder="none"
            disabled={disabled}
            title={BAND_TITLE}
            onChange={(e) => set("catalogueBand", e.target.value)}
            className={INPUT_CLASS}
          />,
        )}
      </div>

      <div className={BLOCK_CLASS}>
        {captioned(
          "Aperture radius",
          <DecimalField
            data-field="radiusText"
            min={0.001}
            value={form.radiusText}
            disabled={disabled}
            onChange={(text) => set("radiusText", text)}
            className={INPUT_CLASS}
          />,
        )}
        {captioned(
          "Radius unit",
          <select
            data-field="radiusUnit"
            value={form.radiusUnit}
            disabled={disabled}
            onChange={(e) => set("radiusUnit", e.target.value as CompareRadiusUnit)}
            className={SELECT_CLASS}
          >
            {nameOptions(RADIUS_UNITS)}
          </select>,
        )}
        <div className="col-span-2">
          <Toggle
            label="Local sky annulus"
            checked={form.localSky}
            accent="cyan"
            disabled={disabled}
            onChange={(checked) => set("localSky", checked)}
          />
        </div>
        {form.localSky &&
          captioned(
            `Sky annulus inner (${form.radiusUnit})`,
            <DecimalField
              data-field="annulusInnerText"
              min={0.001}
              value={form.annulusInnerText}
              disabled={disabled}
              onChange={(text) => set("annulusInnerText", text)}
              className={INPUT_CLASS}
            />,
          )}
        {form.localSky &&
          captioned(
            `Sky annulus outer (${form.radiusUnit})`,
            <DecimalField
              data-field="annulusOuterText"
              min={0.001}
              value={form.annulusOuterText}
              disabled={disabled}
              onChange={(text) => set("annulusOuterText", text)}
              className={INPUT_CLASS}
            />,
          )}
      </div>

      <div className="space-y-1" data-compare-cuts>
        <div className="text-[9px] text-zinc-500">Cuts: a row is compared only when it passes every one</div>
        {form.cuts.map((cut, i) => (
          <div key={i} className="flex items-center gap-1" data-cut={i}>
            <select
              data-field="cutColumn"
              value={cut.column}
              disabled={disabled}
              onChange={(e) => setCut(i, { ...cut, column: e.target.value })}
              className={SELECT_CLASS}
            >
              <option value="">choose</option>
              {nameOptions(columnOptions(columns, cut.column, false))}
            </select>
            <select
              data-field="cutOp"
              value={cut.op}
              disabled={disabled}
              onChange={(e) => setCut(i, { ...cut, op: e.target.value as CompareCutOp })}
              className={SELECT_CLASS}
            >
              {COMPARE_CUT_OPS.map((op) => (
                <option key={op} value={op}>
                  {op}
                </option>
              ))}
            </select>
            <input
              type="text"
              data-field="cutValue"
              value={cut.value}
              placeholder="value"
              disabled={disabled}
              onChange={(e) => setCut(i, { ...cut, value: e.target.value })}
              className={INPUT_CLASS}
            />
            <button
              type="button"
              aria-label={`Remove cut ${i + 1}`}
              disabled={disabled}
              onClick={() => onChange({ ...form, cuts: form.cuts.filter((_, j) => j !== i) })}
              className={CUT_BUTTON_CLASS}
            >
              Remove
            </button>
          </div>
        ))}
        <button
          type="button"
          data-action="add-cut"
          disabled={disabled}
          onClick={() => onChange({ ...form, cuts: [...form.cuts, { ...NEW_CUT }] })}
          className={CUT_BUTTON_CLASS}
        >
          Add a cut
        </button>
      </div>

      <div className={BLOCK_CLASS}>
        {captioned(
          "Minimum S/N",
          <DecimalField
            data-field="minSnrText"
            optional
            min={0}
            value={form.minSnrText}
            placeholder="none"
            disabled={disabled}
            title={SNR_TITLE}
            onChange={(text) => set("minSnrText", text)}
            className={INPUT_CLASS}
          />,
        )}
        {captioned(
          "Maximum S/N",
          <DecimalField
            data-field="maxSnrText"
            optional
            min={0}
            value={form.maxSnrText}
            placeholder="none"
            disabled={disabled}
            title={SNR_TITLE}
            onChange={(text) => set("maxSnrText", text)}
            className={INPUT_CLASS}
          />,
        )}
        {captioned(
          "Nearest neighbour (arcsec)",
          <DecimalField
            data-field="minNeighbourText"
            optional
            min={0.001}
            value={form.minNeighbourText}
            placeholder="none"
            disabled={disabled}
            title={NEIGHBOUR_TITLE}
            onChange={(text) => set("minNeighbourText", text)}
            className={INPUT_CLASS}
          />,
        )}
        <div className="col-span-2">
          <Toggle
            label="Exclude flagged pixels"
            checked={form.excludeDq}
            accent="cyan"
            disabled={disabled}
            onChange={(checked) => set("excludeDq", checked)}
          />
        </div>
      </div>
    </div>
  );
}

export default memo(CompareFormFields);
