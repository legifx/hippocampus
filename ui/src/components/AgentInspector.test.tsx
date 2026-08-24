/**
 * The Agent Inspector, against a fixture daemon.
 *
 * What is worth asserting here is not that React renders — it is the three claims M11 §8 makes
 * about this panel: that it shows what a machine believes, that a person can delete one of
 * those beliefs, and that it does not poll while it is shut. The third is the one that decays
 * silently: a panel that costs a core is a panel that gets switched off, and this project has a
 * recorded case of exactly that (D59).
 */

import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentInspector, type InspectorTab } from "./AgentInspector";
import { setTransport, type Memory } from "../ipc";

const MEMORY: Memory = {
  node_id: 7,
  scope: { kind: "agent", id: "claude" },
  kind: "failure",
  text: "building with --release before the corpus exists produces an empty goldset",
  confidence: 0.8,
  usefulness: -0.15,
  helped_count: 0,
  noise_count: 2,
  retrieved_count: 9,
  human_confirmed: false,
  actor: "agent:claude",
  ts: 1_700_000_000,
  salience: 0.2,
  pinned: false,
};

/** A second memory, so that superseding has something to supersede *with*. */
const REPLACEMENT: Memory = {
  ...({} as Memory),
  node_id: 8,
  scope: { kind: "agent", id: "claude" },
  kind: "fact",
  text: "the corpus is generated before the goldset runs, which is what fixes it",
  confidence: 0.9,
  usefulness: 0,
  helped_count: 0,
  noise_count: 0,
  retrieved_count: 0,
  human_confirmed: false,
  actor: "human",
  ts: 1_700_000_100,
  salience: 0.1,
  pinned: false,
};

type Call = { command: string; args: Record<string, unknown> };

let calls: Call[] = [];
let forgotten: number[] = [];
let superseded: [number, number][] = [];

function fixture(): void {
  calls = [];
  forgotten = [];
  superseded = [];
  setTransport({
    async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      calls.push({ command, args: args ?? {} });
      switch (command) {
        case "agent_sessions":
          return {
            sessions: [
              {
                id: "s-1",
                name: "claude",
                task: "tidy the downloads folder",
                started_at: 1_700_000_000,
                calls: 12,
                refusals: 1,
                memories: 3,
              },
            ],
            open: 1,
            basis: "the 1 most recent agent sessions, newest first; 1 still running",
          } as T;
        case "audit_recent":
          return {
            entries: args?.refusedOnly
              ? [
                  {
                    id: 2,
                    ts: 1_700_000_000,
                    actor: "agent:claude",
                    tool: "fs.write",
                    args_json: "{}",
                    result_summary: "outside the allow-list",
                    allowed: false,
                  },
                ]
              : [
                  {
                    id: 1,
                    ts: 1_700_000_000,
                    actor: "agent:claude",
                    tool: "brain.context",
                    args_json: "{}",
                    result_summary: "6 items",
                    allowed: true,
                  },
                ],
          } as T;
        case "agent_status":
          return {
            enabled: true,
            indexing_paused: false,
            write_allowlist: ["/tmp/work"],
            require_dry_run: true,
          } as T;
        case "memories":
          return {
            memories: forgotten.includes(MEMORY.node_id) ? [] : [MEMORY, REPLACEMENT],
            basis: "every memory in the index, in every scope",
          } as T;
        case "forget_memory":
          forgotten.push(Number(args?.memoryId));
          return { changed: true } as T;
        case "supersede_memory":
          superseded.push([Number(args?.old), Number(args?.new)]);
          return { changed: true } as T;
        default:
          throw new Error(`the fixture has no answer for ${command}`);
      }
    },
    listen: async () => () => {},
    assetUrl: (path: string) => path,
  });
}

function Harness({ open, tab }: { open: boolean; tab: InspectorTab }) {
  return <AgentInspector open={open} tab={tab} onTab={() => {}} onClose={() => {}} />;
}

describe("AgentInspector", () => {
  beforeEach(() => {
    fixture();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows nothing and asks for nothing while it is shut", async () => {
    render(<Harness open={false} tab="live" />);
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(calls).toEqual([]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("does not poll a tab that is not showing", async () => {
    // The memory tab loads once. If the live poll were armed regardless of the tab, this would
    // grow every two seconds — which is the shape of the bug that costs a core.
    render(<Harness open tab="memory" />);
    await waitFor(() => expect(screen.getByText(/empty goldset/)).toBeTruthy());
    const after = calls.length;
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(calls.length).toBe(after);
    expect(calls.some((call) => call.command === "agent_sessions")).toBe(false);
  });

  it("shows what a machine believes, with what it cost and what it earned", async () => {
    render(<Harness open tab="memory" />);
    await waitFor(() => expect(screen.getByText(/empty goldset/)).toBeTruthy());

    // The kind, the writer, and the three counts that say whether it has ever been worth its
    // place: helped, noise, and how often it was shown at all.
    expect(screen.getByText("Fehlschlag")).toBeTruthy();
    expect(screen.getByText("agent:claude")).toBeTruthy();
    expect(screen.getByText("0/2/9")).toBeTruthy();
    expect(screen.getByText("-0.15")).toBeTruthy();
  });

  it("deletes a memory only after the deletion is confirmed", async () => {
    const { getAllByText, getByText } = render(<Harness open tab="memory" />);
    await waitFor(() => expect(screen.getByText(/empty goldset/)).toBeTruthy());

    // One click arms it and deletes nothing. A memory is durable and a mis-click is not a
    // reason to lose one. The first row is the one the fixture puts first.
    await act(async () => {
      getAllByText("Löschen")[0]!.dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true }),
      );
    });
    expect(forgotten).toEqual([]);

    await act(async () => {
      getByText("Wirklich löschen").dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true }),
      );
    });
    await waitFor(() => expect(forgotten).toEqual([MEMORY.node_id]));
    await waitFor(() => expect(screen.queryByText(/empty goldset/)).toBeNull());
  });

  // D78 gives the human three verbs and the first version of this panel wired one of them.
  it("supersedes one memory with another, in two clicks", async () => {
    const { getAllByText, getByText } = render(<Harness open tab="memory" />);
    await waitFor(() => expect(screen.getByText(/empty goldset/)).toBeTruthy());

    // Click one: the memory that is out of date. Nothing has happened yet.
    await act(async () => {
      getAllByText("Ersetzen")[0]!.dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true }),
      );
    });
    expect(superseded).toEqual([]);
    expect(screen.getByText(/Welche Erinnerung ersetzt/)).toBeTruthy();

    // Click two: the one that replaces it. Only the *other* rows offer it, because a memory
    // cannot supersede itself.
    await act(async () => {
      getByText("Ersetzt sie").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    await waitFor(() => expect(superseded).toEqual([[MEMORY.node_id, REPLACEMENT.node_id]]));
    // Superseding is not deleting: the history is the reason it exists.
    expect(forgotten).toEqual([]);
  });

  it("asks the daemon for refusals rather than sieving them out of everything", async () => {
    render(<Harness open tab="policy" />);
    await waitFor(() => expect(screen.getByText("outside the allow-list")).toBeTruthy());
    expect(
      calls.some(
        (call) => call.command === "audit_recent" && call.args.refusedOnly === true,
      ),
    ).toBe(true);
  });

  it("says plainly when the surface is switched off", async () => {
    render(<Harness open tab="policy" />);
    await waitFor(() => expect(screen.getByText("an")).toBeTruthy());
  });
});
