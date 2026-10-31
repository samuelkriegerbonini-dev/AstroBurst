import type { CalibrationMasters } from "../components/stacking/CalibrationPanel";
import type { CalibrationState } from "../components/stacking/StackingTab";

export function withCalibrationFrames(state: CalibrationState, masters: CalibrationMasters): CalibrationState {
  return {
    ...state,
    darkPaths: masters.darkPaths,
    flatPaths: masters.flatPaths,
    biasPaths: masters.biasPaths,
    flatDarkPaths: masters.flatDarkPaths,
  };
}
