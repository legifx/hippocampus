import { forwardRef, type ReactNode } from "react";
import { motion, type HTMLMotionProps } from "framer-motion";
import { cn } from "../../cn";
import { kitSpring, prefersReducedMotion } from "../../motion";
import { useSpecular } from "../../useSpecular";

/* =====================================================================
   GlassButton · the kit's button
   ---------------------------------------------------------------------
   variant  glass   translucent, specular highlight follows the cursor
            solid   ink pill (near-white in this dark theme)
            ghost   invisible until hover (toolbar icons)
            raised  convex "printed" pill (active / toggled states)
            danger  destructive ghost
            blue    tinted glass (primary-but-soft)
   ===================================================================== */

export type GlassVariant = "glass" | "solid" | "ghost" | "raised" | "danger" | "blue";
export type GlassSize = "sm" | "md" | "lg";
export type GlassShape = "pill" | "rounded";

export const buttonSizes: Record<GlassSize, string> = {
  sm: "gb-sm",
  md: "gb-md",
  lg: "gb-lg",
};

export const buttonShapes: Record<GlassShape, string> = {
  pill: "gb-shape-pill",
  rounded: "gb-shape-rounded",
};

interface VariantSpec {
  cls: string;
  /** Whether the cursor-following highlight applies (the kit's `glass-specular`). */
  specular: boolean;
}

export const buttonVariants: Record<GlassVariant, VariantSpec> = {
  glass: { cls: "glass glass-sheen glass-specular gb-glass", specular: true },
  solid: { cls: "solid glass-sheen gb-solid", specular: false },
  ghost: { cls: "gb-ghost", specular: false },
  raised: { cls: "raised gb-raised", specular: false },
  danger: { cls: "gb-danger", specular: false },
  blue: { cls: "glass glass-sheen glass-specular gb-blue", specular: true },
};

export interface GlassButtonProps extends Omit<HTMLMotionProps<"button">, "ref" | "children"> {
  variant?: GlassVariant;
  size?: GlassSize;
  shape?: GlassShape;
  /** Leading icon */
  icon?: ReactNode;
  /** Trailing element (chevron, kbd, count) */
  trailing?: ReactNode;
  /** Force the "raised" look — for toggled / selected states */
  active?: boolean;
  children?: ReactNode;
}

export const GlassButton = forwardRef<HTMLButtonElement, GlassButtonProps>(function GlassButton(
  {
    variant = "glass",
    size = "md",
    shape = "pill",
    icon,
    trailing,
    active = false,
    className,
    children,
    onPointerMove,
    disabled,
    ...rest
  },
  ref,
) {
  const specular = useSpecular();
  const still = prefersReducedMotion();
  const hasLabel = children !== null && children !== undefined && children !== false;
  const spec = buttonVariants[active ? "raised" : variant];

  return (
    <motion.button
      ref={ref}
      type="button"
      disabled={disabled}
      whileTap={disabled || still ? undefined : { scale: 0.97 }}
      transition={kitSpring("springSnappy")}
      onPointerMove={(e) => {
        if (spec.specular) specular.onPointerMove(e);
        onPointerMove?.(e);
      }}
      className={cn("gb", buttonSizes[size], buttonShapes[shape], spec.cls, className)}
      {...rest}
    >
      {icon ? <span className="gb-icon">{icon}</span> : null}
      {hasLabel ? <span className="gb-label">{children}</span> : null}
      {trailing ? <span className="gb-trailing">{trailing}</span> : null}
    </motion.button>
  );
});

/* ---- Icon-only button ------------------------------------------------ */

export interface GlassIconButtonProps extends Omit<GlassButtonProps, "icon" | "trailing"> {
  /** Accessible name (also used as the tooltip) */
  label: string;
}

export const GlassIconButton = forwardRef<HTMLButtonElement, GlassIconButtonProps>(
  function GlassIconButton({ variant = "ghost", size = "md", label, className, children, ...rest }, ref) {
    return (
      <GlassButton
        ref={ref}
        variant={variant}
        size={size}
        aria-label={label}
        title={label}
        icon={children}
        className={cn("gb-icon-only", className)}
        {...rest}
      />
    );
  },
);
