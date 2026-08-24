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
declare module "@agent-view/snapshot" {
  /**
   * Check a snapshot before anything is drawn from it.
   *
   * Used here for the reason `ui-agent`'s own entry point uses it: a daemon that answers with
   * something unexpected should produce one legible sentence rather than a black canvas with a
   * stack trace behind it.
   */
  export function assertSnapshot(value: unknown): unknown;
}

declare module "@agent-view/scene" {
  export class BrainScene {
    constructor(options: { container: HTMLElement; snapshot: unknown });
    /**
     * Draw one frame, and return the seconds since the last one.
     *
     * **The scene does not drive itself.** The standalone view calls this from its own
     * `requestAnimationFrame` loop; anything embedding the scene has to do the same. It was
     * missing from this declaration, so the Agent Inspector could not call it and did not — and
     * a canvas that is built, sized and never drawn is black, with no error, with four thousand
     * nodes behind it. That is the failure this file's own comment says cannot happen, and it
     * happened: a narrow boundary hides a *missing call* as effectively as it hides a wrong one.
     */
    render(onFrame?: (dt: number) => void): number;
    setLayout(mode: "tree" | "shells", animate?: boolean): void;
    /** Send a travelling light along a node's edges. Ignored for a node not in the scene. */
    pulse(nodeId: number, speed?: number): void;
    select(nodeId: number | null): void;
    /** Put the camera back where it started, framing everything. */
    resetCamera(): void;
    dispose(): void;
  }
}
