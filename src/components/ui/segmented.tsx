import type * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Two to four short choices, all visible at once. The track and the raised
 * active item come from `.u-segmented` in index.css.
 */
export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
  disabled,
  label,
  className,
}: {
  value: T;
  options: readonly { value: T; label: React.ReactNode; disabled?: boolean }[];
  onChange: (next: T) => void;
  disabled?: boolean;
  /** Names the group for assistive tech. */
  label: string;
  className?: string;
}): React.ReactElement {
  return (
    <SegmentedGroup label={label} className={className}>
      {options.map((option) => (
        <SegmentedItem
          key={option.value}
          active={value === option.value}
          disabled={disabled || option.disabled}
          onClick={() => {
            if (option.value !== value) onChange(option.value);
          }}
        >
          {option.label}
        </SegmentedItem>
      ))}
    </SegmentedGroup>
  );
}

/** The track, for a segmented control whose items need more than a label. */
export function SegmentedGroup({
  label,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<"div">, "role"> & {
  label: string;
}): React.ReactElement {
  return (
    <div
      role="group"
      aria-label={label}
      className={cn("u-segmented flex shrink-0", className)}
      data-slot="segmented"
      {...props}
    >
      {children}
    </div>
  );
}

export function SegmentedItem({
  active,
  className,
  ...props
}: Omit<React.ComponentProps<"button">, "type"> & {
  active: boolean;
}): React.ReactElement {
  return (
    <button
      type="button"
      className={cn("u-segmented-item disabled:opacity-45", className)}
      data-active={active}
      aria-pressed={active}
      data-slot="segmented-item"
      {...props}
    />
  );
}
