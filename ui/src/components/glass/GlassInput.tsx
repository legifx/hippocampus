import { forwardRef, useRef, type InputHTMLAttributes, type ReactNode } from "react";
import { cn } from "../../cn";
import { Kbd } from "./GlassSurface";
import { SearchIcon, XIcon } from "./icons";

/* =====================================================================
   GlassInput / GlassSearch · recessed pill input
   ---------------------------------------------------------------------
   One box, one type: the padding, the height and the gap are in `em` against the step the
   field is set in (`.gi-md` and friends, `src/kit.css`), so a wider window makes the field
   and the text grow together instead of the text floating in a box that is the wrong size
   for it.
   ===================================================================== */

export interface GlassInputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  icon?: ReactNode;
  trailing?: ReactNode;
  size?: "sm" | "md" | "lg";
  containerClassName?: string;
}

const sizeClass = { sm: "gi-sm", md: "gi-md", lg: "gi-lg" } as const;

export const GlassInput = forwardRef<HTMLInputElement, GlassInputProps>(function GlassInput(
  { icon, trailing, size = "md", className, containerClassName, ...rest },
  ref,
) {
  /* The kit's shell is a `<label>`, which gives click-to-focus for free — and names the field a
     second time whenever the window also puts a visible label beside it (the naming field, the
     inspector's compose field): two labels for one input, and the browser suite's strict locator
     caught exactly that. The shell is a plain box now; each field carries its own name (an
     `aria-label`, or the visible label's `htmlFor`), and the click-to-focus the label gave is
     written here instead. */
  const shell = useRef<HTMLInputElement | null>(null);
  const setShell = (el: HTMLInputElement | null) => {
    shell.current = el;
    if (typeof ref === "function") ref(el);
    else if (ref) ref.current = el;
  };
  return (
    <div
      className={cn("gi well", sizeClass[size], containerClassName)}
      onClick={(event) => {
        const target = event.target as HTMLElement;
        if (target.closest("button") || target.closest("input")) return;
        shell.current?.focus();
      }}
    >
      {icon ? <span className="gi-icon">{icon}</span> : null}
      <input ref={setShell} className={cn("gi-input", className)} {...rest} />
      {trailing}
    </div>
  );
});

export interface GlassSearchProps extends Omit<GlassInputProps, "onChange" | "value"> {
  value: string;
  onChange: (value: string) => void;
  /** e.g. "Strg K" — shown as a key cap while the field is empty */
  shortcut?: string;
  onShortcutClick?: () => void;
  /**
   * Accessible name of the clear button. The kit hard-codes an English "Clear search"; this
   * window is German throughout, and the name is what a screen reader reads and what the tests
   * reach for, so it is a prop with the German default rather than a string inside the port.
   */
  clearLabel?: string;
  /**
   * Called after the field has been emptied. The kit's own button only clears; this window also
   * wants the caret back in the field, because the person who clicked clear was in the middle of
   * a query and the next keystroke belongs in the box.
   */
  onClear?: () => void;
}

export const GlassSearch = forwardRef<HTMLInputElement, GlassSearchProps>(function GlassSearch(
  { value, onChange, shortcut, onShortcutClick, clearLabel = "Eingabe löschen", onClear, placeholder = "Suchen", ...rest },
  ref,
) {
  return (
    <GlassInput
      ref={ref}
      icon={<SearchIcon />}
      value={value}
      placeholder={placeholder}
      onChange={(e) => onChange(e.target.value)}
      trailing={
        value ? (
          <button
            type="button"
            className="gi-clear"
            aria-label={clearLabel}
            title={clearLabel}
            onClick={() => {
              onChange("");
              onClear?.();
            }}
          >
            <XIcon />
          </button>
        ) : shortcut ? (
          <button type="button" tabIndex={-1} className="gi-shortcut" onClick={onShortcutClick}>
            <Kbd>{shortcut}</Kbd>
          </button>
        ) : null
      }
      {...rest}
    />
  );
});
