import { describe, expect, it } from "vitest";

import type { SaveLocations } from "@/bindings/SaveLocations";
import { browseStart, displayPath, fillStoredNames, storedNameFor } from "./savePaths";

const locations: SaveLocations = {
  savesRoot: null,
  staging: null,
  prefixHome: "/home/me/Games/prefixes/7/pfx/drive_c/users/steamuser",
  prefixDriveC: "/home/me/Games/prefixes/7/pfx/drive_c",
  installDir: "/home/me/Games/Celeste",
  home: "/home/me",
  savesInPrefix: true,
  detected: [],
  suggested: [],
};

describe("storedNameFor", () => {
  it("is the same for the same title, and unique among taken names", () => {
    expect(storedNameFor("The Witcher 3: Wild Hunt", [])).toBe("/gameyfin/custom/the-witcher-3-wild-hunt");
    expect(storedNameFor("Pokémon", [])).toBe("/gameyfin/custom/pokémon");
    expect(storedNameFor("Celeste", ["/gameyfin/custom/celeste"])).toBe("/gameyfin/custom/celeste-2");
    expect(storedNameFor("???", [])).toBe("/gameyfin/custom/game");
  });
});

describe("fillStoredNames", () => {
  it("fills blanks in order and leaves typed names and empty rows alone", () => {
    const rows = fillStoredNames(
      [
        { source: " /a ", target: "" },
        { source: "/b", target: "" },
        { source: "/c", target: "/mine" },
        { source: "", target: "" },
      ],
      "Celeste",
    );
    expect(rows).toEqual([
      { source: "/a", target: "/gameyfin/custom/celeste" },
      { source: "/b", target: "/gameyfin/custom/celeste-2" },
      { source: "/c", target: "/mine" },
      { source: "", target: "" },
    ]);
  });
});

describe("browseStart", () => {
  it("opens inside the prefix only for a game that saves there", () => {
    expect(browseStart(locations)).toBe(locations.prefixHome);
    expect(browseStart({ ...locations, savesInPrefix: false })).toBe("/home/me");
    expect(browseStart({ ...locations, prefixHome: null })).toBe(locations.prefixDriveC);
    expect(browseStart(null)).toBeUndefined();
  });
});

describe("displayPath", () => {
  it("shows prefix paths as Windows ones and home paths with a tilde", () => {
    expect(displayPath(`${locations.prefixHome}/Documents/My Games`, locations)).toBe(
      "C:\\users\\steamuser\\Documents\\My Games",
    );
    expect(displayPath("/home/me/.local/share/Celeste", locations)).toBe("~/.local/share/Celeste");
    expect(displayPath("/home/meadow", locations)).toBe("/home/meadow");
  });
});
