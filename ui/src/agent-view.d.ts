/**
 * What this window uses of the Agent View, declared rather than imported.
 *
 * The scene itself lives in `ui-agent/src` and Vite resolves `@agent-view/*` there at build
 * time (see `vite.config.ts`). Type-checking those files *here* would compile somebody else's
 * project under this one's compiler options — `noUncheckedIndexedAccess` in particular, which
 * `ui-agent` does not set — and produce two hundred errors about code that is green in its own
 * package and has its own tests.
 *
 * So the boundary is declared instead, and it is deliberately narrow: a constructor, a
 * `dispose`, and a snapshot this window does not look inside. Anything wider would be a second
 * copy of `ui-agent`'s types, which is the duplication the alias exists to avoid.
 *
 * The cost is real and worth naming: a change to `BrainScene`'s constructor will not be caught
 * here at compile time. It will be caught by `ui-agent`'s own `tsc --noEmit`, and by the fact
 * that this tab fails loudly and visibly rather than silently — which is why the tab reports
 * what went wrong instead of showing an empty canvas.
 */
declare module "@agent-view/scene" {
  export class BrainScene {
    constructor(options: { container: HTMLElement; snapshot: unknown });
    setLayout(mode: "tree" | "shells", animate?: boolean): void;
    dispose(): void;
  }
}
