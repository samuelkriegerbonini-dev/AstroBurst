const STAGE_FIELDS = 5;
const LANE_POSITION = /^\d+\/\d+$/;
const LIGHT_COUNT = /^\d+$/;

const STAGE_TEXT: Record<string, string> = {
  masters: "Building calibration masters",
  saving: "Saving masters",
  complete: "Finishing",
};

function stageFields(stage: string): string[] {
  const parts = stage.split(":");
  if (parts.length <= STAGE_FIELDS) return parts;
  return [...parts.slice(0, STAGE_FIELDS - 1), parts.slice(STAGE_FIELDS - 1).join(":")];
}

export function pipelineProgressText(stage: string): string {
  if (!stage) return "Starting the pipeline...";
  const [step, , position, lights, label] = stageFields(stage);
  const known = STAGE_TEXT[step];
  if (known) return known;
  if (label === undefined || !LANE_POSITION.test(position) || !LIGHT_COUNT.test(lights)) return step;
  const count = Number(lights);
  return `${label} ${position}: ${step} ${count} light${count === 1 ? "" : "s"}`;
}
