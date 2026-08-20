/**
 * The breath layer (design system §6).
 *
 * One `position: absolute` gradient whose opacity rises and falls, with a period derived from
 * indexing load — six seconds idle, down to 2.4 at full ingest. That is the whole progress
 * indicator this application has, and it replaces a bar deliberately: a bar asks to be read and
 * then waited on, while a heartbeat tells somebody who is looking elsewhere that work is
 * happening.
 *
 * **It is a CSS animation, not a JavaScript one, and that is the point of §6.** The rule says
 * ambient layers are compositor-only. Framer Motion drives values from a `requestAnimationFrame`
 * loop, which means an animation that never ends is a JavaScript callback on every frame for as
 * long as the window is open — the opposite of compositor-only, and enough to make an otherwise
 * responsive interface feel sticky on a renderer that is already working hard. The first version
 * of this file did exactly that.
 */

import { breathPeriod, prefersReducedMotion } from "../motion";

export function Ambient({ load }: { load: number }) {
  // §7: a static layer when the person asked for less motion. Not a slower pulse — a pulse is
  // the thing being objected to.
  if (prefersReducedMotion()) {
    return <div className="hc-breath" style={{ opacity: 0.7 }} aria-hidden="true" />;
  }

  return (
    <div
      className="hc-breath hc-breath-animated"
      aria-hidden="true"
      // The only thing React sets is the period. The animation itself never touches this thread.
      style={{ animationDuration: `${breathPeriod(load)}s` }}
    />
  );
}
