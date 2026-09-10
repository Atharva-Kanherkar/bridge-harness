type Props = {
  eyebrow: string;
  title: React.ReactNode;
  text?: string;
  align?: "left" | "center";
};

export default function SectionHeader({ eyebrow, title, text, align = "left" }: Props) {
  const centered = align === "center";
  return (
    <div className={`reveal ${centered ? "mx-auto flex flex-col items-center text-center" : ""}`}>
      <span className="eyebrow">{eyebrow}</span>
      <h2 className="mt-4 max-w-3xl font-display text-[2.25rem] font-semibold leading-[1.05] tracking-[-0.03em] text-foreground sm:text-[2.875rem]">
        {title}
      </h2>
      {text && <p className={`mt-5 max-w-2xl text-[15.5px] leading-7 text-muted-foreground ${centered ? "mx-auto" : ""}`}>{text}</p>}
    </div>
  );
}
