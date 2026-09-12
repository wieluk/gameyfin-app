import { describe, expect, it } from "vitest";

import { isInDownloads, isInstalled, isLocal, primaryAction } from "./actions";
import * as fixture from "./testing";
import type { GameState } from "@/types";

const entry = fixture.testEntry;

describe("isLocal", () => {
  it("counts everything the user could act on without the server", () => {
    expect(isLocal(entry(fixture.installed()))).toBe(true);
    expect(isLocal(entry(fixture.running()))).toBe(true);
    expect(isLocal(entry({ kind: "downloaded", archivePath: "/a.zip", bytes: 10 }))).toBe(true);
    expect(
      isLocal(entry({ kind: "extracted", path: "/x", setupCandidates: [], archivePresent: true })),
    ).toBe(true);
  });

  it("excludes a game that exists only on the server", () => {
    expect(isLocal(entry({ kind: "not-installed" }))).toBe(false);
  });

  it("excludes transfers with nothing usable on disk yet", () => {
    // These appear in Downloads, but offering them in an offline library would promise
    // something the user cannot finish without the server.
    expect(
      isLocal(entry({ kind: "downloading", receivedBytes: 1, totalBytes: 9, bytesPerSecond: 1 })),
    ).toBe(false);
    expect(isLocal(entry({ kind: "preparing", message: "Setting up" }))).toBe(false);
    expect(isLocal(entry(fixture.failed({ message: "no" })))).toBe(false);
  });

  it("is narrower than the Downloads filter", () => {
    const failed = entry(fixture.failed({ message: "no" }));
    expect(isInDownloads(failed)).toBe(true);
    expect(isLocal(failed)).toBe(false);
  });

  it("agrees with isInstalled for installed games", () => {
    const installed = entry(fixture.installed());
    expect(isInstalled(installed)).toBe(true);
    expect(isLocal(installed)).toBe(true);
  });
});

describe("an installed game with work in progress", () => {
  const preparing = fixture.installed({
    busy: { kind: "preparing", message: "Setting up Wine" },
  });

  it("stays in Installed while its prefix is set up", () => {
    // Setting up Wine must not move a playable game back to Downloads.
    expect(isInstalled(entry(preparing))).toBe(true);
    expect(isInDownloads(entry(preparing))).toBe(false);
    expect(isLocal(entry(preparing))).toBe(true);
  });

  it("offers no Play until the work finishes", () => {
    const action = primaryAction(preparing);
    expect(action.disabled).toBe(true);
    expect(action.label).toBe("Setting up…");
  });

  it("offers Play again once only a failure is left", () => {
    const state = fixture.installed({
      busy: { kind: "failed", message: "the installer exited with code 1" },
    });
    expect(primaryAction(state).label).toBe("Play");
    expect(primaryAction(state).disabled).toBe(false);
  });
});

describe("a game that fails to start", () => {
  const failedLaunch = fixture.failed({ stage: "launch" });

  it("stays in Installed, because it is", () => {
    // Downloads would be neither true nor useful: the files are on disk, and
    // re-downloading them was never the fix.
    expect(isInstalled(entry(failedLaunch))).toBe(true);
    expect(isInDownloads(entry(failedLaunch))).toBe(false);
    expect(isLocal(entry(failedLaunch))).toBe(true);
  });

  it("offers Play rather than Retry download", () => {
    expect(primaryAction(failedLaunch).label).toBe("Play");
  });

  it("leaves the other failures where they were", () => {
    for (const stage of ["download", "extract", "install"] as const) {
      const state: GameState = fixture.failed({ stage });
      expect(isInDownloads(entry(state))).toBe(true);
      expect(isInstalled(entry(state))).toBe(false);
    }
  });
});
