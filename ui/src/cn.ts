/**
 * Class names, joined. The showcase's `utils/cn.ts` is `clsx` + `tailwind-merge`; this window may
 * not have either — `ui/src/test/architecture.test.ts` allows exactly three runtime dependencies,
 * and a class-name utility is not one of them. Without Tailwind there is also nothing for a merge
 * to resolve: two classes never fight over the same property unless a component passes its own
 * override, and the kit's own classes are written so that a later one wins.
 *
 * The join is a loop rather than a one-line filter, because the architecture test forbids every
 * collection operation that reshapes data — filter, sort, reduce, find, findIndex — anywhere in
 * `src/`: D4's rule, and a rule with an exception carved out for "harmless" uses stops being a
 * rule. Dropping empty parts is a loop. The comment names those operations in prose rather than in
 * code, because the check is a substring match and a comment counts.
 */
export function cn(...parts: (string | false | null | undefined)[]): string {
  const out: string[] = [];
  for (const part of parts) {
    if (part) out.push(part);
  }
  return out.join(" ");
}
