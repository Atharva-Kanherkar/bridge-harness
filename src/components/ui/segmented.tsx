import * as React from "react";
import { cn } from "@/lib/utils";
import { Tip } from "@/components/ui/tooltip";

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
  tabs = false,
  ...props
}: Omit<React.ComponentProps<"div">, "role"> & {
  label: string;
  /** Section switcher: a tablist whose items are tabs. */
  tabs?: boolean;
}): React.ReactElement {
  return (
    <div
      role={tabs ? "tablist" : "group"}
      aria-label={label}
      className={cn("u-segmented flex shrink-0", className)}
      data-slot="segmented"
      {...props}
    >
      {children}
    </div>
  );
}

type SegmentedItemProps = Omit<React.ComponentProps<"button">, "type"> & {
  active: boolean;
  /** Inside a `tabs` group: a tab, selected rather than pressed. */
  tab?: boolean;
  /** Inside a `SegmentedRadio`: a radio, checked rather than pressed. */
  radio?: boolean;
};

export const SegmentedItem = React.forwardRef<HTMLButtonElement, SegmentedItemProps>(function SegmentedItem(
  { active, tab = false, radio = false, className, ...props },
  ref,
): React.ReactElement {
  return (
    <button
      ref={ref}
      type="button"
      className={cn("u-segmented-item inline-flex items-center justify-center gap-1 disabled:opacity-45", className)}
      data-active={active}
      role={tab ? "tab" : radio ? "radio" : undefined}
      aria-selected={tab ? active : undefined}
      aria-checked={radio ? active : undefined}
      aria-pressed={tab || radio ? undefined : active}
      data-slot="segmented-item"
      {...props}
    />
  );
});

/**
 * A segmented control that is a radio group: one tab stop (the checked item,
 * or the first when none is), arrows wrap and pick, Home and End jump. Each
 * option may carry a hover hint and extra data attributes.
 */
export function SegmentedRadio<T extends string>({
  label,
  value,
  options,
  onChange,
  disabled,
  className,
  itemClassName,
  describedBy,
}: {
  label: string;
  value: T | undefined;
  options: readonly {
    value: T;
    label: React.ReactNode;
    hint?: string;
    data?: Record<`data-${string}`, string>;
  }[];
  onChange?: (next: T) => void;
  disabled?: boolean;
  className?: string;
  itemClassName?: string;
  describedBy?: string;
}): React.ReactElement {
  const index = options.findIndex((option) => option.value === value);
  const inert = disabled || !onChange;
  const items = React.useRef<Array<HTMLButtonElement | null>>([]);
  const onKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>, i: number) => {
    if (inert) return;
    const step =
      event.key === "ArrowRight" || event.key === "ArrowDown" ? 1
      : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1
      : 0;
    if (!step && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const count = options.length;
    const next =
      event.key === "Home" ? 0
      : event.key === "End" ? count - 1
      : index < 0 ? (step > 0 ? 0 : count - 1)
      : (i + step + count) % count;
    if (next !== index) onChange?.(options[next].value);
    items.current[next]?.focus();
  };
  return (
    <div
      role="radiogroup"
      aria-label={label}
      aria-describedby={describedBy}
      className={cn("u-segmented flex shrink-0", className)}
      data-slot="segmented"
    >
      {options.map((option, i) => {
        const on = i === index;
        return (
          <Tip key={option.value} label={option.hint}>
            <SegmentedItem
              ref={(node) => {
                items.current[i] = node;
              }}
              radio
              active={on}
              tabIndex={on || (index < 0 && i === 0) ? 0 : -1}
              disabled={inert}
              className={itemClassName}
              onClick={() => {
                if (!on) onChange?.(option.value);
              }}
              onKeyDown={(event) => onKeyDown(event, i)}
              {...option.data}
            >
              {option.label}
            </SegmentedItem>
          </Tip>
        );
      })}
    </div>
  );
}
