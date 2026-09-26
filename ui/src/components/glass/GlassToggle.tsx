import type { MouseEvent } from "react";
import { motion } from "framer-motion";
import { cn } from "../../cn";
import { kitSpring, prefersReducedMotion } from "../../motion";

/* =====================================================================
   GlassToggle · recessed track, raised knob (tints blue when on)
   GlassCheckbox · well when off, ink when on, check path draws in
   ===================================================================== */

export interface GlassToggleProps {
  checked: boolean;
  onChange: (next: boolean, event: MouseEvent<HTMLButtonElement>) => void;
  size?: "sm" | "md";
  disabled?: boolean;
  label?: string;
  className?: string;
}

const toggleDims = {
  sm: { w: 36, h: 20, k: 16 },
  md: { w: 46, h: 26, k: 22 },
};

export function GlassToggle({
  checked,
  onChange,
  size = "md",
  disabled,
  label,
  className,
}: GlassToggleProps) {
  const d = toggleDims[size];
  const pad = (d.h - d.k) / 2;
  const travel = d.w - d.k - pad * 2;
  const still = prefersReducedMotion();

  return (
    <motion.button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      initial={false}
      animate={checked ? "on" : "off"}
      whileTap={disabled || still ? undefined : "tap"}
      onClick={(e) => onChange(!checked, e)}
      className={cn("tgl well", checked && "tgl-on", className)}
      style={{ width: d.w, height: d.h }}
    >
      <motion.span
        className="tgl-knob raised"
        variants={{
          on: { x: travel, scaleX: 1 },
          off: { x: 0, scaleX: 1 },
          tap: { scaleX: 1.18 },
        }}
        transition={kitSpring("springSnappy")}
        style={{
          top: pad,
          left: pad,
          width: d.k,
          height: d.k,
          originX: checked ? 1 : 0,
          backgroundImage: checked
            ? "linear-gradient(180deg, color-mix(in srgb, var(--hc-accent-blue) 78%, white), var(--hc-accent-blue))"
            : undefined,
        }}
      />
    </motion.button>
  );
}

export interface GlassCheckboxProps {
  checked: boolean;
  indeterminate?: boolean;
  onChange: (next: boolean) => void;
  size?: number;
  className?: string;
  label?: string;
}

export function GlassCheckbox({
  checked,
  indeterminate = false,
  onChange,
  size = 20,
  className,
  label,
}: GlassCheckboxProps) {
  const on = checked || indeterminate;
  const still = prefersReducedMotion();
  return (
    <motion.button
      type="button"
      role="checkbox"
      aria-checked={indeterminate ? "mixed" : checked}
      aria-label={label}
      whileTap={still ? undefined : { scale: 0.88 }}
      transition={kitSpring("springSnappy")}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!checked);
      }}
      onDoubleClick={(e) => e.stopPropagation()}
      className={cn("cbx", on ? "solid" : "well cbx-off", className)}
      style={{ width: size, height: size }}
    >
      <svg
        viewBox="0 0 16 16"
        width={size * 0.62}
        height={size * 0.62}
        fill="none"
        stroke="currentColor"
        strokeWidth={2.4}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <motion.path
          d={indeterminate ? "M3.5 8h9" : "M3.2 8.4l3.1 3.1 6.5-7"}
          initial={false}
          animate={{ pathLength: on ? 1 : 0, opacity: on ? 1 : 0 }}
          transition={still ? { duration: 0 } : { duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
        />
      </svg>
    </motion.button>
  );
}
