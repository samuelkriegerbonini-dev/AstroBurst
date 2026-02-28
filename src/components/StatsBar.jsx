import { motion } from "framer-motion";
import { formatThroughput } from "../utils/format";

function AnimatedNumber({ value }) {
  return (
    <motion.span
      key={value}
      initial={{ y: -8, opacity: 0 }}
      animate={{ y: 0, opacity: 1 }}
      transition={{ duration: 0.2 }}
      className="inline-block"
    >
      {value}
    </motion.span>
  );
}

export default function StatsBar({ stats, elapsed, formatted, isComplete }) {
  const throughput = formatThroughput(
    stats.done + stats.failed,
    elapsed
  );

  return (
    <div className="flex items-center justify-between text-xs">
      <div className="flex items-center gap-3 text-zinc-400">
        <span className="font-mono">
          <AnimatedNumber value={stats.total} /> files
        </span>
        <span className="text-zinc-700">|</span>
        <span className="font-mono text-green-400">
          <AnimatedNumber value={stats.done} /> done
        </span>
        {stats.failed > 0 && (
          <>
            <span className="text-zinc-700">|</span>
            <span className="font-mono text-red-400">
              <AnimatedNumber value={stats.failed} /> error
            </span>
          </>
        )}
        <span className="text-zinc-700">|</span>
        <span className="font-mono text-zinc-300">{formatted}</span>
      </div>

      <div className="flex items-center gap-3 text-zinc-500">
        {isComplete && (
          <motion.span
            initial={{ opacity: 0, scale: 0.8 }}
            animate={{ opacity: 1, scale: 1 }}
            className="text-green-400 font-medium"
          >
            Complete
          </motion.span>
        )}
        <span className="font-mono">{throughput}</span>
      </div>
    </div>
  );
}
