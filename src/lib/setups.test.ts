import { describe, expect, it } from "vitest";
import type { SetupProgram } from "@/bindings/SetupProgram";
import {
  arranged,
  coversEverything,
  defaultSelection,
  remaining,
  reordered,
  runOrder,
  toggled,
} from "./setups";

function setup(path: string, overrides: Partial<SetupProgram> = {}): SetupProgram {
  return {
    path,
    role: "dlc",
    superseded: false,
    installed: false,
    silent: true,
    recommended: true,
    matched: true,
    ...overrides,
  };
}

const gogGame = [
  setup("setup_game.exe", { role: "game" }),
  setup("patch_game_1.0_to_1.1.exe", { role: "patch", superseded: true, recommended: false }),
  setup("setup_game_dlc.exe"),
  setup("autorun.exe", { role: "other", silent: false, recommended: false }),
];

describe("setup selection", () => {
  it("starts from what an automatic install would run", () => {
    expect(defaultSelection(gogGame)).toEqual(["setup_game.exe", "setup_game_dlc.exe"]);
  });

  it("keeps a ticked program in the order they run, whichever list it was ticked in", () => {
    const withOther = [...gogGame, setup("tools/fix.exe", { role: "other", matched: false })];
    const start = ["setup_game_dlc.exe"];
    expect(toggled(withOther, start, "tools/fix.exe", true)).toEqual([
      "setup_game_dlc.exe",
      "tools/fix.exe",
    ]);
    expect(toggled(withOther, start, "setup_game.exe", true)).toEqual([
      "setup_game.exe",
      "setup_game_dlc.exe",
    ]);
    expect(toggled(withOther, start, "setup_game_dlc.exe", false)).toEqual([]);
  });

  it("runs the ticked setups in the order the user arranged", () => {
    const order = ["setup_game_dlc.exe", "setup_game.exe"];
    const paths = arranged(gogGame, order).map((s) => s.path);
    // Never arranged, so after the rest in plan order.
    expect(paths).toEqual([
      "setup_game_dlc.exe",
      "setup_game.exe",
      "patch_game_1.0_to_1.1.exe",
      "autorun.exe",
    ]);
    expect(runOrder(arranged(gogGame, order), ["setup_game.exe", "setup_game_dlc.exe"])).toEqual(
      order,
    );
    expect(arranged(gogGame, null)).toBe(gogGame);
  });

  it("moves a shown row without disturbing the hidden ones", () => {
    const all = ["a", "hidden", "b", "c"];
    expect(reordered(all, ["a", "b", "c"], 2, 0)).toEqual(["c", "hidden", "a", "b"]);
    expect(reordered(all, ["a", "b", "c"], 0, 1)).toEqual(["b", "hidden", "a", "c"]);
  });

  it("keeps the download while something worth installing is left out", () => {
    expect(coversEverything(gogGame, ["setup_game.exe", "setup_game_dlc.exe"])).toBe(true);
    expect(coversEverything(gogGame, ["setup_game.exe"])).toBe(false);
  });

  it("leaves installed, superseded and unexplained setups out of what remains", () => {
    const installed = gogGame.map((s) =>
      s.role === "game" ? { ...s, installed: true } : s,
    );
    expect(remaining(installed).map((s) => s.path)).toEqual(["setup_game_dlc.exe"]);
  });
});
