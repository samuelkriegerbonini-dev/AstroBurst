import { useEffect, useState, useRef, memo } from "react";

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
    isCircle: Math.random() > 0.5,
  }));
}

function Confetti({ show }) {
  const [particles, setParticles] = useState([]);
  const timerRef = useRef(null);

  useEffect(() => {
    if (show) {
      setParticles(generateParticles());
      timerRef.current = setTimeout(() => setParticles([]), 3000);
      return () => clearTimeout(timerRef.current);
    }
    setParticles([]);
  }, [show]);

  if (particles.length === 0) return null;

  return (
    <div className="fixed inset-0 pointer-events-none z-50 overflow-hidden">
      {particles.map((p) => (
        <div
          key={p.id}
          className="confetti-particle absolute"
          style={{
            left: `${p.x}vw`,
            top: -20,
            width: p.size,
            height: p.size,
            backgroundColor: p.color,
            borderRadius: p.isCircle ? "50%" : "2px",
            animationDuration: `${p.duration}s`,
            animationDelay: `${p.delay}s`,
          }}
        />
      ))}
    </div>
  );
}

export default memo(Confetti);
