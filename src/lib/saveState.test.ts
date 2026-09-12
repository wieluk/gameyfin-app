import { describe as group, expect, it } from "vitest";

import { describe, outcomeOf } from "./saveState";

group("outcomeOf", () => {
  it("says a backup worked", () => {
    expect(outcomeOf({ kind: "in-sync", lastSyncedAt: null })).toEqual({
      text: "Backed up.",
      ok: true,
    });
  });

  it("names the title it searched under when nothing was found", () => {
    // The title tells the user whether the game was matched to the wrong entry.
    const outcome = outcomeOf({
      kind: "nothing-to-back-up",
      title: "Death Must Die",
      known: true,
    });

    expect(outcome.ok).toBe(false);
    expect(outcome.text).toContain("Death Must Die");
    expect(outcome.text).toContain("Set folders");
  });

  it("points at the database when the title is not in it", () => {
    const outcome = outcomeOf({
      kind: "nothing-to-back-up",
      title: "Some Obscure Game",
      known: false,
    });

    expect(outcome.text).toContain("no entry");
    expect(outcome.text).toContain("game database");
  });

  it("passes a failure message through rather than inventing one", () => {
    expect(outcomeOf({ kind: "failed", message: "the disk is full" })).toEqual({
      text: "the disk is full",
      ok: false,
    });
  });
});

group("describe", () => {
  it("does not blame the server when sync is switched off here", () => {
    // A local checkbox being off must not read as the server lacking save sync.
    const off = describe({ kind: "off" });
    const unsupported = describe({ kind: "unsupported" });

    expect(off.text).not.toEqual(unsupported.text);
    expect(off.text).toContain("Settings");
    expect(off.text).not.toContain("server");
    expect(unsupported.text).toContain("server");
  });

  it("distinguishes an unknown title from an empty one", () => {
    const known = describe({ kind: "nothing-to-back-up", title: "Celeste", known: true });
    const unknown = describe({ kind: "nothing-to-back-up", title: "Celeste", known: false });

    expect(known.text).not.toEqual(unknown.text);
    expect(unknown.text).toContain("database");
  });
});
