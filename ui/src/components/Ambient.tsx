/**
 * The ambient layer (design system §6.1): the breath.
 *
 * One `position: absolute` gradient whose opacity rises and falls, with a period derived from
 * indexing load — six seconds idle, down to 2.4 at full ingest. That is the whole progress indicator
 * this application has, and it replaces a bar deliberately: a bar asks to be read and then waited on,
 * while a heartbeat tells somebody who is looking elsewhere that work is happening.
 *
 * **It is a CSS animation, not a JavaScript one, and that is the point of §6.** The rule says
 * ambient layers are compositor-only. Framer Motion drives values from a `requestAnimationFrame`
 * loop, which means an animation that never ends is a JavaScript callback on every frame for as
 * long as the window is open — the opposite of compositor-only, and enough to make an otherwise
 * responsive interface feel sticky on a renderer that is already working hard. The first version
 * of this file did exactly that.
 *
 * **The layer is alone on purpose.** A canvas of three coloured glows stood here for one session —
 * the kit's backdrop, measured and declined for its frame cost (D114) and then removed altogether
 * because the owner's verdict was about the concept rather than the cost: the glow took the
 * *minimalism* out of the interface (D115). The light this window has is this one gradient, and
 * what it carries is the indexing heartbeat, not decoration.
 */

import { breathPeriod, prefersReducedMotion } from "../motion";

export function Ambient({ load }: { load: number }) {
  // §7: a static layer when the person asked for less motion. Not a slower pulse — a pulse is the
  // thing being objected to.
  if (prefersReducedMotion()) {
    return <div className="hc-breath" style={{ opacity: 0.7 }} aria-hidden="true" />;
  }

  // Nothing is being indexed, so there is nothing to beat about.
  if (load <= 0) {
    return <div className="hc-breath" aria-hidden="true" />;
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
