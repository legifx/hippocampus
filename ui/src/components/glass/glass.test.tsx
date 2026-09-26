/**
 * The design kit's contract, as behaviour rather than as markup.
 *
 * The kit is ported from the showcase's design-kit page (`ui/inspo/…/src/glass/*`), and the point
 * of porting it instead of copying a look is that the behaviour arrives with it: a segmented
 * control that keeps its tab order and answers the arrow keys, a field whose clear button empties
 * it and gives the caret back, a toggle that says what it is to a screen reader. Anything the kit
 * promises here, this window can stop writing out by hand — so these are the promises worth
 * pinning down.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { GlassButton, GlassIconButton } from "./GlassButton";
import { GlassSearch } from "./GlassInput";
import { GlassSegmented } from "./GlassSegmented";
import { GlassCheckbox, GlassToggle } from "./GlassToggle";

describe("the kit's segmented control", () => {
  const options = [
    { value: "counts", label: "Zahlen", title: "Was gezählt wurde" },
    { value: "rules", label: "Regeln" },
    { value: "audit", label: "Protokoll" },
  ];

  it("is a tablist, and says which tab is the value", () => {
    render(<GlassSegmented options={options} value="rules" onChange={() => {}} />);
    expect(screen.getByRole("tablist")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Regeln" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tab", { name: "Zahlen" }).getAttribute("aria-selected")).toBe("false");
  });

  it("only the selected tab is in the tab order, so Tab moves past the strip", () => {
    render(<GlassSegmented options={options} value="rules" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: "Regeln" }).getAttribute("tabindex")).toBe("0");
    expect(screen.getByRole("tab", { name: "Zahlen" }).getAttribute("tabindex")).toBe("-1");
  });

  it("reports a press", () => {
    const onChange = vi.fn();
    render(<GlassSegmented options={options} value="rules" onChange={onChange} />);
    fireEvent.click(screen.getByRole("tab", { name: "Protokoll" }));
    expect(onChange).toHaveBeenCalledWith("audit");
  });

  it("walks the strip with the arrow keys, once around", () => {
    const onChange = vi.fn();
    render(<GlassSegmented options={options} value="audit" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole("tablist"), { key: "ArrowRight" });
    expect(onChange).toHaveBeenLastCalledWith("counts");
    fireEvent.keyDown(screen.getByRole("tablist"), { key: "ArrowLeft" });
    expect(onChange).toHaveBeenLastCalledWith("rules");
  });

  it("keeps the hint as a tooltip rather than as another line of text", () => {
    render(<GlassSegmented options={options} value="counts" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: "Zahlen" }).getAttribute("title")).toBe(
      "Was gezählt wurde",
    );
  });
});

describe("the kit's search field", () => {
  it("reports every keystroke", () => {
    const onChange = vi.fn();
    render(<GlassSearch value="" onChange={onChange} aria-label="Suche" />);
    fireEvent.change(screen.getByLabelText("Suche"), { target: { value: "mod" } });
    expect(onChange).toHaveBeenCalledWith("mod");
  });

  it("offers the shortcut while it is empty and the way out once it is not", () => {
    const { rerender } = render(
      <GlassSearch value="" onChange={() => {}} shortcut="Strg K" aria-label="Suche" />,
    );
    expect(screen.getByText("Strg K")).toBeTruthy();
    expect(screen.queryByLabelText("Eingabe löschen")).toBeNull();

    rerender(<GlassSearch value="mod" onChange={() => {}} shortcut="Strg K" aria-label="Suche" />);
    expect(screen.queryByText("Strg K")).toBeNull();
    expect(screen.getByLabelText("Eingabe löschen")).toBeTruthy();
  });

  it("empties itself and gives the caret back, which is why the button exists", () => {
    const onChange = vi.fn();
    const onClear = vi.fn();
    render(<GlassSearch value="mod" onChange={onChange} onClear={onClear} aria-label="Suche" />);
    fireEvent.click(screen.getByLabelText("Eingabe löschen"));
    expect(onChange).toHaveBeenCalledWith("");
    expect(onClear).toHaveBeenCalled();
  });

  it("calls the shortcut cap, and only when there is something to call", () => {
    const onShortcutClick = vi.fn();
    const { rerender } = render(
      <GlassSearch value="" onChange={() => {}} shortcut="Strg K" onShortcutClick={onShortcutClick} />,
    );
    fireEvent.click(screen.getByText("Strg K"));
    expect(onShortcutClick).toHaveBeenCalled();

    rerender(<GlassSearch value="" onChange={() => {}} />);
    expect(screen.queryByText("Strg K")).toBeNull();
  });
});

describe("the kit's controls", () => {
  it("a toggle is a switch and says whether it is on", () => {
    const onChange = vi.fn();
    const { rerender } = render(<GlassToggle checked={false} onChange={onChange} label="Ordner" />);
    expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
    fireEvent.click(screen.getByRole("switch"));
    /* The event comes with the value: the kit's switch hands both to the caller. */
    expect(onChange).toHaveBeenCalledWith(true, expect.anything());

    rerender(<GlassToggle checked onChange={onChange} label="Ordner" />);
    expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("true");
  });

  it("a checkbox is a checkbox, and a click may not also tick it by itself", () => {
    const onChange = vi.fn();
    render(<GlassCheckbox checked={false} onChange={onChange} label="Gewählt" />);
    const box = screen.getByRole("checkbox");
    fireEvent.click(box);
    expect(onChange).toHaveBeenCalledWith(true);
    /* The component reports and the caller decides — a press that also flipped its own state would
       be a second source of truth for the same fact. */
    expect(box.getAttribute("aria-checked")).toBe("false");
  });

  it("a disabled button does not fire", () => {
    const onClick = vi.fn();
    render(
      <GlassButton disabled onClick={onClick}>
        Löschen
      </GlassButton>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Löschen" }));
    expect(onClick).not.toHaveBeenCalled();
  });

  it("an icon button has to say what it does, because there is no word to read", () => {
    render(
      <GlassIconButton label="Schließen">
        <span />
      </GlassIconButton>,
    );
    expect(screen.getByRole("button", { name: "Schließen" })).toBeTruthy();
  });
});
