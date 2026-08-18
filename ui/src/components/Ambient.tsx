/**
 * The breath layer (design system §6).
 *
 * One `position: absolute` gradient animating opacity and nothing else. Its period is derived
 * from indexing load — six seconds idle, down to 2.4 at full ingest — and that is the whole
 * progress indicator this application has.
 *
 * It replaces a progress bar deliberately. A bar asks to be read and then to be waited on; a
 * heartbeat says "something is happening" to a person who is looking somewhere else, and a
 * background indexer is exactly the kind of work nobody should be waiting on.
 */

import { motion } from "framer-motion";
import { breathPeriod, prefersReducedMotion } from "../motion";

export function Ambient({ load }: { load: number }) {
  // §7: a static layer when the person asked for less motion. Not a slower pulse — a pulse is
  // the thing being objected to.
  if (prefersReducedMotion()) {
    return <div className="hc-breath" style={{ opacity: 0.7 }} aria-hidden="true" />;
  }

  return (
    <motion.div
      className="hc-breath"
      aria-hidden="true"
      animate={{ opacity: [0.4, 1, 0.4] }}
      transition={{
        duration: breathPeriod(load),
        repeat: Infinity,
        ease: "easeInOut",
      }}
    />
  );
}
