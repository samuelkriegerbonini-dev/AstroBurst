import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";

const COLORS = ["#3b82f6", "#22c55e", "#a855f7", "#eab308", "#ef4444"];
const PARTICLE_COUNT = 40;

function generateParticles() {
  return Array.from({ length: PARTICLE_COUNT }, (_, i) => ({
    id: i,
    x: Math.random() * 100,
    color: COLORS[Math.floor(Math.random() * COLORS.length)],
    size: Math.random() * 6 + 4,
    delay: Math.random() * 0.5,
    duration: Math.random() * 1.5 + 1.5,
  }));
}

export default function Confetti({ show }) {
  const [particles, setParticles] = useState([]);

  useEffect(() => {
    if (show) {
      setParticles(generateParticles());
      const timer = setTimeout(() => setParticles([]), 3000);
      return () => clearTimeout(timer);
    }
    setParticles([]);
  }, [show]);

  return (
    <AnimatePresence>
      {particles.length > 0 && (
        <div className="fixed inset-0 pointer-events-none z-50 overflow-hidden">
          {particles.map((p) => (
            <motion.div
              key={p.id}
              initial={{
                x: `${p.x}vw`,
                y: -20,
                rotate: 0,
                opacity: 1,
              }}
              animate={{
                y: "110vh",
                rotate: 720,
                opacity: 0,
              }}
              transition={{
                duration: p.duration,
                delay: p.delay,
                ease: "easeOut",
              }}
              style={{
                position: "absolute",
                width: p.size,
                height: p.size,
                backgroundColor: p.color,
                borderRadius: Math.random() > 0.5 ? "50%" : "2px",
              }}
            />
          ))}
        </div>
      )}
    </AnimatePresence>
  );
}
