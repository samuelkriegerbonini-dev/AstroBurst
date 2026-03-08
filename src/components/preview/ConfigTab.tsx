import { lazy, Suspense, memo } from "react";
import { Loader2 } from "lucide-react";

const ConfigPanel = lazy(() => import("../ConfigPanel"));

function ConfigTabInner() {
  return (
    <Suspense
      fallback={
        <div className="flex items-center justify-center py-12">
          <Loader2 size={20} className="animate-spin text-zinc-500" />
        </div>
      }
    >
      <ConfigPanel />
    </Suspense>
  );
}

export default memo(ConfigTabInner);
