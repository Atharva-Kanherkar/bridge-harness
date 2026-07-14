export function SpaceBackground() {
  return (
    <div aria-hidden className="pointer-events-none fixed inset-0 overflow-hidden">
      <div className="absolute inset-0 bg-[#020204]" />
      <div
        className="absolute -top-[20%] -left-[10%] h-[60rem] w-[60rem] rounded-full opacity-[0.12]"
        style={{
          background: "radial-gradient(circle, rgba(99,102,241,0.4) 0%, rgba(45,212,191,0.15) 40%, transparent 70%)",
          filter: "blur(100px)",
          animation: "space-drift-a 40s ease-in-out infinite alternate",
        }}
      />
      <div
        className="absolute -bottom-[20%] -right-[10%] h-[50rem] w-[50rem] rounded-full opacity-[0.10]"
        style={{
          background: "radial-gradient(circle, rgba(167,139,250,0.35) 0%, rgba(45,212,191,0.1) 50%, transparent 70%)",
          filter: "blur(120px)",
          animation: "space-drift-b 50s ease-in-out infinite alternate",
        }}
      />
      <StarField />
    </div>
  );
}

function StarField() {
  const stars = [
    { x: 12, y: 8, s: 1.2, o: 0.5, d: 3.2 },
    { x: 28, y: 15, s: 0.8, o: 0.3, d: 4.1 },
    { x: 45, y: 5, s: 1.5, o: 0.6, d: 2.8 },
    { x: 62, y: 22, s: 0.6, o: 0.25, d: 5.5 },
    { x: 78, y: 12, s: 1.0, o: 0.4, d: 3.8 },
    { x: 88, y: 30, s: 0.7, o: 0.35, d: 4.5 },
    { x: 5, y: 35, s: 1.3, o: 0.55, d: 3.0 },
    { x: 22, y: 42, s: 0.5, o: 0.2, d: 6.0 },
    { x: 38, y: 28, s: 1.1, o: 0.45, d: 3.5 },
    { x: 55, y: 38, s: 0.9, o: 0.3, d: 4.2 },
    { x: 72, y: 48, s: 1.4, o: 0.5, d: 2.9 },
    { x: 85, y: 55, s: 0.6, o: 0.2, d: 5.8 },
    { x: 15, y: 58, s: 1.0, o: 0.4, d: 3.6 },
    { x: 32, y: 65, s: 0.8, o: 0.25, d: 4.8 },
    { x: 48, y: 52, s: 1.2, o: 0.55, d: 3.1 },
    { x: 68, y: 68, s: 0.7, o: 0.3, d: 4.4 },
    { x: 82, y: 72, s: 1.1, o: 0.45, d: 3.3 },
    { x: 8, y: 78, s: 0.9, o: 0.35, d: 4.0 },
    { x: 25, y: 82, s: 1.3, o: 0.5, d: 2.7 },
    { x: 42, y: 75, s: 0.6, o: 0.2, d: 5.2 },
    { x: 58, y: 85, s: 1.0, o: 0.4, d: 3.7 },
    { x: 75, y: 88, s: 0.8, o: 0.3, d: 4.6 },
    { x: 92, y: 80, s: 1.4, o: 0.55, d: 2.6 },
    { x: 18, y: 92, s: 0.7, o: 0.25, d: 5.0 },
  ];

  return (
    <>
      {stars.map(star => (
        <div
          key={`${star.x}-${star.y}`}
          className="absolute rounded-full bg-white"
          style={{
            left: `${star.x}%`,
            top: `${star.y}%`,
            width: `${star.s}px`,
            height: `${star.s}px`,
            opacity: star.o,
            animation: `star-twinkle ${star.d}s ease-in-out infinite alternate`,
            boxShadow: star.s > 1 ? `0 0 ${star.s * 2}px rgba(255,255,255,0.3)` : "none",
          }}
        />
      ))}
    </>
  );
}
