/**
 * Whether anybody can see this window.
 *
 * The overlay is resident: it is started once at login and spends almost all of its life
 * hidden. A hidden GTK window does not suspend the page inside it — WebKit keeps running the
 * document, so animations keep animating and intervals keep firing — which means everything
 * this interface does periodically, it does forever, for nobody.
 *
 * What that cost before it was measured: with WebKit dropped to software compositing (which is
 * what `WEBKIT_DISABLE_DMABUF_RENDERER=1` does), the breath layer alone held 99% of a CPU core
 * with the window off screen. The renderer is no longer disabled, so that particular bill is
 * paid in the compositor now — but the four-second poll behind it was still two full table
 * scans in the daemon, round the clock, for an animation nobody was looking at.
 *
 * `document.visibilityState` is not used because it does not change: the document is visible as
 * far as WebKit is concerned, whatever the window manager has done with the window around it.
 * The shell knows, so the shell says.
 */

import { useEffect, useState } from "react";
import { ipc } from "../ipc";

export function useVisibility(): boolean {
  // Starts true: the first thing that happens is a paint, and a window that assumed it was
  // hidden would come up frozen and only recover when it was next summoned.
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    let disposed = false;
    let off: (() => void) | null = null;

    ipc
      .onVisibility((next) => !disposed && setVisible(next))
      .then((unlisten) => {
        if (disposed) unlisten();
        else off = unlisten;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      off?.();
    };
  }, []);

  return visible;
}
