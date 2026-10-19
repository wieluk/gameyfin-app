import { describe, expect, it } from "vitest";

import { cardSubtitle } from "./GameCard";
import { installed, testEntry, testGame } from "@/lib/testing";

const GiB = 1024 ** 3;

describe("cardSubtitle", () => {
  it("says playtime is playtime", () => {
    expect(cardSubtitle(testEntry(installed(), { minutesPlayed: 135 }))).toBe("2 h 15 min played");
  });

  it("counts playtime from another PC on a game not installed here", () => {
    const entry = testEntry({ kind: "not-installed" }, { minutesPlayed: 45 });
    expect(cardSubtitle(entry)).toBe("45 min played");
  });

  it("does not offer a download size for a game already installed", () => {
    const entry = testEntry(installed(), { game: testGame({ metadata: { fileSize: 2 * GiB, originalIds: null } }) });
    expect(cardSubtitle(entry)).toBe("Not played yet");
  });

  it("names the size of a game not installed as its download", () => {
    const entry = testEntry({ kind: "not-installed" }, { game: testGame({ metadata: { fileSize: 2 * GiB, originalIds: null } }) });
    expect(cardSubtitle(entry)).toBe("2.0 GB download");
  });

  it("does not show 0 B when the server reports no size", () => {
    expect(cardSubtitle(testEntry({ kind: "not-installed" }))).toBe("Not installed");
  });
});
