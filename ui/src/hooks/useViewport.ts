/**
 * The window's width, as a number the layout can branch on.
 *
 * CSS media queries would do most of this, and the layout uses them too — but which *panes
 * exist* is a React decision, not a CSS one. A pane hidden with `display: none` still mounts,
 * still fetches, and still holds DOM nodes, and at 400 px in a tiling column that is exactly
 * the work worth not doing.
 *
 * A `ResizeObserver` on the document element rather than a `resize` listener: under a tiling
 * compositor a column can be resized without the window ever firing `resize`.
 */

import { useEffect, useState } from "react";

export function useViewport(): number {
  const [width, setWidth] = useState(() =>
    typeof window === "undefined" ? 1200 : window.innerWidth,
  );

  useEffect(() => {
    if (typeof window === "undefined") return;

    const update = () => setWidth(window.innerWidth);
    update();

    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(update);
      observer.observe(document.documentElement);
      return () => observer.disconnect();
    }

    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  return width;
}
