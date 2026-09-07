import { cn } from "@/lib/utils";

export function BridgeMark({ size = "md", className }: { size?: "sm" | "md" | "lg"; className?: string }) {
  const sizeClasses = {
    sm: "text-xl",
    md: "text-[1.75rem] sm:text-[2rem]",
    lg: "text-[2.25rem] sm:text-[2.75rem]",
  } as const;

  return (
    <p className={cn("font-display tracking-[-0.03em] text-foreground", sizeClasses[size], className)}>
      bridge
    </p>
  );
}
