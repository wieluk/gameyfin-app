// @vitest-environment node
import { describe, expect, it } from "vitest";
import { groupCommits, toMarkdown, toReleasesXml } from "./release-notes.mjs";

describe("groupCommits", () => {
  it("sorts conventional subjects into sections and drops plumbing", () => {
    const sections = groupCommits([
      "fix(saves): make Proton saves travel between machines",
      "docs: add user guide",
      "feat(library): add filters",
      "feat!: drop the old config",
      "chore: trim comments",
    ]);
    expect(sections).toEqual([
      { title: "New", items: ["Library: add filters", "Drop the old config"] },
      { title: "Fixed", items: ["Saves: make Proton saves travel between machines"] },
    ]);
  });

  it("keeps a non-conventional subject as a change, but not a merge", () => {
    expect(groupCommits(["fix flatpak version in release", "Merge branch 'x'"])).toEqual([
      { title: "Changes", items: ["Fix flatpak version in release"] },
    ]);
  });
});

describe("toMarkdown", () => {
  it("says so when nothing user facing changed", () => {
    expect(toMarkdown([])).toBe("Maintenance release.\n");
  });
});

describe("toReleasesXml", () => {
  it("escapes entries, links the release and marks pre-releases", () => {
    const xml = toReleasesXml(
      [
        {
          tag: "v1.0.0-beta.1",
          version: "1.0.0-beta.1",
          date: "2026-09-13",
          sections: [{ title: "Fixed", items: ["Saves: <b> & more"] }],
        },
      ],
      "https://github.com/wieluk/gameyfin-app",
    );
    expect(xml).toContain('<release version="1.0.0-beta.1" date="2026-09-13" type="development">');
    expect(xml).toContain(
      '<url type="details">https://github.com/wieluk/gameyfin-app/releases/tag/v1.0.0-beta.1</url>',
    );
    expect(xml).toContain("<li>Saves: &lt;b&gt; &amp; more</li>");
  });
});
