/**
 * The selection model, which is the thing a file manager is unusable without.
 *
 * These are behaviour tests on the pure part — the range arithmetic — plus the hook driven
 * through `act`. Both matter: the arithmetic is where an off-by-one lives, and the hook is
 * where "the anchor moved when it should not have" lives.
 */

import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { rangeBetween, useSelection } from "./useSelection";

const sorted = (set: Set<number>) => [...set].sort((a, b) => a - b);

describe("rangeBetween", () => {
  it("includes both ends, in either direction", () => {
    expect(sorted(rangeBetween(2, 5))).toEqual([2, 3, 4, 5]);
    expect(sorted(rangeBetween(5, 2))).toEqual([2, 3, 4, 5]);
    expect(sorted(rangeBetween(3, 3))).toEqual([3]);
  });
});

describe("useSelection", () => {
  it("starts on the first row with it selected", () => {
    const { result } = renderHook(() => useSelection(10, "list"));
    expect(result.current.state.cursor).toBe(0);
    expect(sorted(result.current.state.indices)).toEqual([0]);
  });

  it("moves one row at a time and replaces the selection", () => {
    const { result } = renderHook(() => useSelection(10, "list"));

    act(() => result.current.move("down", false));
    act(() => result.current.move("down", false));

    expect(result.current.state.cursor).toBe(2);
    expect(sorted(result.current.state.indices)).toEqual([2]);
  });

  it("cannot walk off either end", () => {
    const { result } = renderHook(() => useSelection(3, "list"));

    act(() => result.current.move("up", false));
    expect(result.current.state.cursor).toBe(0);

    for (let i = 0; i < 8; i++) act(() => result.current.move("down", false));
    expect(result.current.state.cursor).toBe(2);
  });

  it("extends from the anchor, and the anchor does not move", () => {
    const { result } = renderHook(() => useSelection(10, "list"));

    act(() => result.current.select(4));
    act(() => result.current.move("down", true));
    act(() => result.current.move("down", true));
    expect(sorted(result.current.state.indices)).toEqual([4, 5, 6]);

    // Back up past the anchor: the range flips rather than growing in both directions.
    act(() => result.current.move("up", true));
    act(() => result.current.move("up", true));
    act(() => result.current.move("up", true));
    expect(sorted(result.current.state.indices)).toEqual([3, 4]);
    expect(result.current.state.anchor).toBe(4);
  });

  it("shift-clicking twice picks a range rather than growing one", () => {
    const { result } = renderHook(() => useSelection(10, "list"));

    act(() => result.current.select(2));
    act(() => result.current.extendTo(6));
    expect(sorted(result.current.state.indices)).toEqual([2, 3, 4, 5, 6]);

    // The second shift-click measures from the same anchor, which is what makes it a
    // correction rather than an addition.
    act(() => result.current.extendTo(4));
    expect(sorted(result.current.state.indices)).toEqual([2, 3, 4]);
  });

  it("ctrl toggles one row and takes the anchor with it", () => {
    const { result } = renderHook(() => useSelection(10, "list"));

    act(() => result.current.select(1));
    act(() => result.current.toggle(5));
    act(() => result.current.toggle(8));
    expect(sorted(result.current.state.indices)).toEqual([1, 5, 8]);

    act(() => result.current.toggle(5));
    expect(sorted(result.current.state.indices)).toEqual([1, 8]);

    // And a shift-extension now measures from the last ctrl-clicked row.
    expect(result.current.state.anchor).toBe(5);
  });

  it("takes everything and gives it back", () => {
    const { result } = renderHook(() => useSelection(4, "list"));

    act(() => result.current.selectAll());
    expect(sorted(result.current.state.indices)).toEqual([0, 1, 2, 3]);

    act(() => result.current.clear());
    // Clearing keeps the cursor's row: a file manager with nothing selected at all has no
    // answer to "what does Enter open".
    expect(sorted(result.current.state.indices)).toEqual([0]);
  });

  it("forgets everything when the list changes underneath it", () => {
    const { result, rerender } = renderHook(
      ({ key }: { key: string }) => useSelection(10, key),
      { initialProps: { key: "one" } },
    );

    act(() => result.current.selectAll());
    expect(result.current.state.indices.size).toBe(10);

    // A different directory. Keeping index 3 selected would mean deleting whatever happens to
    // be third in the one somebody just navigated into.
    rerender({ key: "two" });
    expect(sorted(result.current.state.indices)).toEqual([0]);
    expect(result.current.state.cursor).toBe(0);
  });

  it("holds nothing in an empty list", () => {
    const { result } = renderHook(() => useSelection(0, "empty"));
    expect(result.current.state.indices.size).toBe(0);
    // And moving in one does not invent a row.
    act(() => result.current.move("down", false));
    expect(result.current.state.cursor).toBe(0);
    expect(result.current.state.indices.size).toBe(0);
  });
});
