import { describe, expect, it } from "vitest";
import type { SaveSyncProgress } from "@/bindings/SaveSyncProgress";
import { decide } from "./syncWindow";

function at(
  phase: SaveSyncProgress["phase"],
  overrides: Partial<SaveSyncProgress> = {},
): SaveSyncProgress {
  return {
    gameId: 1,
    title: "Celeste",
    moment: "launch",
    phase,
    skippable: true,
    blocking: false,
    ...overrides,
  };
}

const checking = at({ kind: "checking" });
const inSync = at({ kind: "done", state: { kind: "in-sync", lastSyncedAt: null } });
const restored = at({ kind: "restored", files: 1, folders: ["/s"], savedAt: null, device: null });

describe("the save window", () => {
  it("waits before showing a step, in case it is over at once", () => {
    expect(decide(null, checking).kind).toBe("later");
  });

  it("opens for nothing when the outcome is quiet", () => {
    expect(decide(null, inSync).kind).toBe("keep");
    expect(decide(null, at({ kind: "kept-local" })).kind).toBe("keep");
  });

  it("closes out a window already showing progress", () => {
    expect(decide(checking, inSync)).toEqual({ kind: "show", progress: inSync });
    expect(decide(checking, at({ kind: "downloading" })).kind).toBe("show");
  });

  it("shows failures and restores straight away", () => {
    expect(decide(null, at({ kind: "failed", message: "x" })).kind).toBe("show");
    expect(decide(null, restored).kind).toBe("show");
  });

  it("gives way to the save prompt", () => {
    expect(decide(checking, at({ kind: "asking" })).kind).toBe("close");
    expect(decide(null, at({ kind: "asking" })).kind).toBe("close");
    // Another game's message stays.
    expect(decide(restored, at({ kind: "asking" }, { gameId: 2 })).kind).toBe("keep");
  });

  it("does not cover a restore with a quiet outcome", () => {
    expect(decide(restored, inSync).kind).toBe("keep");
  });

  it("speaks up for an exit upload held back", () => {
    expect(decide(null, at({ kind: "skipped" }, { moment: "exit" })).kind).toBe("show");
    expect(decide(null, at({ kind: "skipped" })).kind).toBe("keep");
  });
});
