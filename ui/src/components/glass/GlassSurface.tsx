import { forwardRef, type HTMLAttributes, type ReactNode } from "react";
import { motion, type HTMLMotionProps } from "framer-motion";
import { cn } from "../../cn";
import { kitSpring, prefersReducedMotion } from "../../motion";
import { useSpecular } from "../../useSpecular";

/* =====================================================================
   Surfaces & small atoms
   ---------------------------------------------------------------------
   Ported from `ui/inspo/.../src/glass/GlassSurface.tsx`. The material names are the
   kit's; the class bodies are in `src/glass.css`, and the radii are this project's
   scale (`--hc-r-*`) rather than the kit's inline `rounded-[20px]`.
   ===================================================================== */

export type Material = "glass" | "glass-strong" | "glass-weak" | "raised" | "well" | "solid";

export const materialClass: Record<Material, string> = {
  glass: "glass glass-sheen",
  "glass-strong": "glass glass-strong glass-sheen",
  "glass-weak": "glass glass-weak glass-sheen",
  raised: "raised",
  well: "well",
  solid: "solid",
};

export type PanelRadius = "sm" | "md" | "lg" | "xl" | "pill";

const radiusClass: Record<PanelRadius, string> = {
  sm: "rounded-sm",
  md: "rounded-md",
  lg: "rounded-lg",
  xl: "rounded-xl",
  pill: "rounded-pill",
};

export interface GlassPanelProps extends HTMLAttributes<HTMLDivElement> {
  material?: Material;
  radius?: PanelRadius;
}

/** Static glass container. */
export const GlassPanel = forwardRef<HTMLDivElement, GlassPanelProps>(function GlassPanel(
  { material = "glass", radius = "lg", className, ...rest },
  ref,
) {
  return <div ref={ref} className={cn("panel", materialClass[material], radiusClass[radius], className)} {...rest} />;
});

export interface GlassCardProps extends Omit<HTMLMotionProps<"div">, "ref"> {
  interactive?: boolean;
}

/**
 * Interactive glass card — lifts on hover, presses on tap, specular follows the cursor.
 *
 * `glass-flat`, not `glass`: a card sits *inside* a panel, and two drop shadows stacked on
 * each other read as one muddy edge rather than as two surfaces. The kit draws this same
 * distinction with `.glass-flat`.
 */
export const GlassCard = forwardRef<HTMLDivElement, GlassCardProps>(function GlassCard(
  { interactive = true, className, onPointerMove, ...rest },
  ref,
) {
  const specular = useSpecular();
  const still = prefersReducedMotion();
  return (
    <motion.div
      ref={ref}
      whileHover={interactive && !still ? { y: -2 } : undefined}
      whileTap={interactive && !still ? { scale: 0.985 } : undefined}
      transition={kitSpring("springSnappy")}
      onPointerMove={(e) => {
        specular.onPointerMove(e);
        onPointerMove?.(e);
      }}
      className={cn(
        "card glass glass-flat glass-sheen glass-specular rounded-lg",
        interactive && "card-interactive",
        className,
      )}
      {...rest}
    />
  );
});

/** 1px divider. */
export function Hairline({ className, vertical = false }: { className?: string; vertical?: boolean }) {
  return <div aria-hidden="true" className={cn("hairline", vertical && "is-vertical", className)} />;
}

/** Keyboard key cap. */
export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return <kbd className={cn("kbd well", className)}>{children}</kbd>;
}

/** Count bubble (sidebar badges). */
export function CountBadge({
  children,
  tone = "accent",
  className,
}: {
  children: ReactNode;
  tone?: "accent" | "pink" | "blue";
  className?: string;
}) {
  const tones = { accent: "solid", pink: "tone-pink", blue: "tone-blue" };
  return <span className={cn("count-badge", tones[tone], className)}>{children}</span>;
}

/** Tiny status dot (unread indicator). */
export function StatusDot({
  tone = "pink",
  className,
}: {
  tone?: "pink" | "blue" | "success" | "danger";
  className?: string;
}) {
  return <span aria-hidden="true" className={cn("status-dot", `tone-${tone}`, className)} />;
}

/** 4-point sparkle used as the app mark. */
export function SparkIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className={cn("spark-icon", className)} aria-hidden="true">
      <path d="M12 1.5c.55 5.7 4.8 9.95 10.5 10.5-5.7.55-9.95 4.8-10.5 10.5C11.45 16.8 7.2 12.55 1.5 12 7.2 11.45 11.45 7.2 12 1.5z" />
    </svg>
  );
}
