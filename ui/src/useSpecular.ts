import { useCallback, type PointerEvent } from "react";

/**
 * The design kit's cursor-following specular highlight, ported verbatim: it writes `--mx` / `--my`
 * as percentages and the material's `.glass-specular::after` radial gradient reads them.
 *
 *   const specular = useSpecular();
 *   <button className="glass raised-bezel glass-specular" {...specular} />
 *
 * Used sparingly and deliberately: one element per panel at most. The handler runs on every
 * pointer move over the element, and §8 is explicit that a per-frame effect attached to something
 * that repeats (a row) is what breaks the frame budget — so no row gets one.
 */
export function useSpecular() {
  const onPointerMove = useCallback((e: PointerEvent<HTMLElement>) => {
    const el = e.currentTarget;
    const rect = el.getBoundingClientRect();
    const x = ((e.clientX - rect.left) / rect.width) * 100;
    const y = ((e.clientY - rect.top) / rect.height) * 100;
    el.style.setProperty("--mx", `${x.toFixed(1)}%`);
    el.style.setProperty("--my", `${y.toFixed(1)}%`);
  }, []);

  return { onPointerMove };
}
