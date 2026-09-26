import { useId, type ReactNode } from "react";
import { motion } from "framer-motion";
import { cn } from "../../cn";
import { kitSpring } from "../../motion";

/* =====================================================================
   GlassSegmented · recessed track + sliding "printed" thumb
   ---------------------------------------------------------------------
   thumb    raised  convex pill inside the well (default)
            bezel   raised + double bezel ring, slightly bulging (hero look)
            solid   ink pill
            glass   translucent pill

   The thumb is a single element that moves, not a class that jumps between buttons: it
   carries a shared `layoutId`, so Framer Motion animates the same node from one segment to
   the next. That is the whole reason the segments are `isolate`d and the thumb sits behind
   the label.
   ===================================================================== */

export interface SegmentOption<T extends string> {
  value: T;
  label?: ReactNode;
  icon?: ReactNode;
  disabled?: boolean;
  title?: string;
}

export type SegmentThumb = "raised" | "bezel" | "solid" | "glass";

export interface GlassSegmentedProps<T extends string> {
  options: SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  size?: "sm" | "md" | "lg";
  thumb?: SegmentThumb;
  /** Thin hairlines between inactive segments ("Home | Library | Profile") */
  dividers?: boolean;
  fullWidth?: boolean;
  className?: string;
  "aria-label"?: string;
}

const sizeClass = { sm: "seg-sm", md: "seg-md", lg: "seg-lg" } as const;

const thumbs: Record<SegmentThumb, { cls: string; inset: string }> = {
  raised: { cls: "raised", inset: "seg-thumb-inset" },
  bezel: { cls: "raised-bezel", inset: "seg-thumb-bezel-inset" },
  solid: { cls: "solid seg-thumb-solid", inset: "seg-thumb-inset" },
  glass: { cls: "glass glass-sheen", inset: "seg-thumb-inset" },
};

export function GlassSegmented<T extends string>({
  options,
  value,
  onChange,
  size = "md",
  thumb = "raised",
  dividers = false,
  fullWidth = false,
  className,
  "aria-label": ariaLabel,
}: GlassSegmentedProps<T>) {
  const layoutId = useId();
  /*
   * Which segment is the value, as a loop: findIndex is one of the collection operations
   * `ui/src/test/architecture.test.ts` forbids in `src/` — the frontend renders what the daemon
   * sends and does not reach into collections (D4). Looking up one index is a loop.
   */
  let activeIndex = -1;
  for (let i = 0; i < options.length; i += 1) {
    if (options[i]!.value === value) {
      activeIndex = i;
      break;
    }
  }
  const t = thumbs[thumb];

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      onKeyDown={(event) => {
        const step =
          event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
        if (step === 0 || options.length === 0) return;
        const from = activeIndex === -1 ? 0 : activeIndex;
        const next = (from + step + options.length) % options.length;
        const option = options[next];
        if (!option || option.disabled) return;
        event.preventDefault();
        onChange(option.value);
      }}
      className={cn("seg well", sizeClass[size], thumb === "bezel" && "seg-bezel", fullWidth && "seg-full", className)}
    >
      {options.map((o, i) => {
        const active = i === activeIndex;
        const showDivider = dividers && i > 0 && !active && i - 1 !== activeIndex;
        return (
          <button
            key={o.value}
            type="button"
            role="tab"
            aria-selected={active}
            /* The kit's control leaves the whole strip in the tab order and answers no arrow key.
               A `role="tablist"` without "one tab stop per strip, arrows inside it" is the ARIA
               pattern with its most useful half missing, and this window already had it — so the
               port carries it (docs/06-DESIGN-SYSTEM.md §10 says where the kit was extended). */
            tabIndex={active ? 0 : -1}
            disabled={o.disabled}
            title={o.title}
            onClick={() => onChange(o.value)}
            className="seg-item"
          >
            {active ? (
              <motion.span
                layoutId={layoutId}
                className={cn("seg-thumb", t.inset, t.cls)}
                transition={kitSpring("springSnappy")}
              />
            ) : null}
            <span aria-hidden="true" className={cn("seg-divider", showDivider && "is-shown")} />
            {o.icon}
            {o.label !== undefined && o.label !== null ? <span>{o.label}</span> : null}
          </button>
        );
      })}
    </div>
  );
}
