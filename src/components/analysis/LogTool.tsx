import { lazy, Suspense, memo } from "react";
import { ANALYSIS_SECTION } from "../../utils/analysisSections";
import TabSpinner from "./TabSpinner";

const MeasurementLogPanel = lazy(() => import("./MeasurementLogPanel"));

function LogTool() {
  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="p-3">
        <section id={ANALYSIS_SECTION.log.id}>
          <MeasurementLogPanel />
        </section>
      </div>
    </Suspense>
  );
}

export default memo(LogTool);
