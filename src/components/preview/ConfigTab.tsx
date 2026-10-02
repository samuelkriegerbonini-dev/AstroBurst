import { lazy, Suspense, memo } from "react";
import { Loader2 } from "lucide-react";

const ConfigPanel = lazy(() => import("../ConfigPanel"));

function ConfigTabInner() {
  return (
    <div className="p-3">
      <Suspense
        fallback={
          <div className="flex items-center justify-center py-12">
            <Loader2 size={20} className="animate-spin text-zinc-500" />
          </div>
        }
      >
        <ConfigPanel />
      </Suspense>
    </div>
  );
}

export default memo(ConfigTabInner);
