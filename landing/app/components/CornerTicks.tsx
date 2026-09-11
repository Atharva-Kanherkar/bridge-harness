/** The ticks a technical drawing wears at its corners. */
export default function CornerTicks() {
  const positions = ["-left-1.5 -top-1.5", "-right-1.5 -top-1.5", "-bottom-1.5 -left-1.5", "-bottom-1.5 -right-1.5"];
  return (
    <>
      {positions.map(position => (
        <span key={position} aria-hidden="true" className={`absolute text-[13px] leading-none text-faint-2 ${position}`}>
          +
        </span>
      ))}
    </>
  );
}
