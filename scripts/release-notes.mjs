#!/usr/bin/env node
/**
 * Release notes from conventional commit subjects, for the GitHub release and the Flatpak
 * metainfo. Usage: node scripts/release-notes.mjs markdown|metainfo v1.2.3
 */

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const METAINFO = join(ROOT, "flatpak", "org.gameyfin.Gameyfin.metainfo.xml");

/** Software centers show only the last few, so older releases only lengthen the file. */
const HISTORY = 10;

// Other types (docs, ci, chore, ...) change nothing a user would notice.
const SECTIONS = [
  { title: "New", type: "feat" },
  { title: "Improved", type: "perf" },
  { title: "Fixed", type: "fix" },
];

const capitalize = (text) => text.charAt(0).toUpperCase() + text.slice(1);

const ACRONYMS = new Set(["api", "ci", "ui"]);
const scopeLabel = (scope) => (ACRONYMS.has(scope) ? scope.toUpperCase() : capitalize(scope));

/** Sections with their entries, in display order. A non-conventional subject counts as a change. */
export function groupCommits(subjects) {
  const sections = SECTIONS.map(({ title }) => ({ title, items: [] }));
  const other = [];

  for (const subject of subjects) {
    const match = subject.match(/^([a-z]+)(?:\(([^)]+)\))?!?:\s*(.+)$/);
    if (!match) {
      if (!subject.startsWith("Merge ")) other.push(capitalize(subject));
      continue;
    }
    const [, type, scope, text] = match;
    const index = SECTIONS.findIndex((section) => section.type === type);
    if (index === -1) continue;
    sections[index].items.push(scope ? `${scopeLabel(scope)}: ${text}` : capitalize(text));
  }

  if (other.length > 0) sections.push({ title: "Changes", items: other });
  return sections.filter((section) => section.items.length > 0);
}

export function toMarkdown(sections) {
  if (sections.length === 0) return "Maintenance release.\n";
  return sections
    .map(({ title, items }) => `### ${title}\n\n${items.map((item) => `- ${item}`).join("\n")}\n`)
    .join("\n");
}

const escapeXml = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** The `<releases>` block, newest first. `homepage` links each release to its GitHub page. */
export function toReleasesXml(releases, homepage) {
  const entries = releases.map(({ tag, version, date, sections }) => {
    // AppStream marks pre-releases as development, so software centers do not offer them as stable.
    const type = version.includes("-") ? ' type="development"' : "";
    const body =
      sections.length === 0
        ? ["<p>Maintenance release.</p>"]
        : sections.flatMap(({ title, items }) => [
            `<p>${escapeXml(title)}</p>`,
            "<ul>",
            ...items.map((item) => `  <li>${escapeXml(item)}</li>`),
            "</ul>",
          ]);
    return [
      `    <release version="${escapeXml(version)}" date="${date}"${type}>`,
      ...(homepage
        ? [`      <url type="details">${escapeXml(`${homepage}/releases/tag/${tag}`)}</url>`]
        : []),
      "      <description>",
      ...body.map((line) => `        ${line}`),
      "      </description>",
      "    </release>",
    ].join("\n");
  });
  return `<releases>\n${entries.join("\n")}\n  </releases>`;
}

function git(...args) {
  // Sorts v1.0.0-beta before v1.0.0, as a version comparison would.
  return execFileSync("git", ["-c", "versionsort.suffix=-", ...args], {
    cwd: ROOT,
    encoding: "utf8",
  }).trim();
}

const lines = (text) => text.split("\n").filter(Boolean);

/** `tag` and the releases before it in its history, newest first. Needs a full clone. */
export function releasesUpTo(tag, limit = HISTORY) {
  const tags = lines(git("tag", "--list", "v*", "--merged", tag, "--sort=-v:refname"));
  return tags.slice(0, limit).map((name, i) => {
    const range = tags[i + 1] ? `${tags[i + 1]}..${name}` : name;
    return {
      tag: name,
      version: name.replace(/^v/, ""),
      date: git("log", "-1", "--format=%cs", name),
      sections: groupCommits(lines(git("log", "--no-merges", "--format=%s", range))),
    };
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [mode, tag] = process.argv.slice(2);
  if (!["markdown", "metainfo"].includes(mode) || !tag) {
    console.error("Usage: node scripts/release-notes.mjs markdown|metainfo v1.2.3");
    process.exit(1);
  }

  const releases = releasesUpTo(tag);
  if (releases[0]?.tag !== tag) {
    console.error(`${tag} is not a v* tag.`);
    process.exit(1);
  }

  if (mode === "markdown") {
    process.stdout.write(toMarkdown(releases[0].sections));
  } else {
    const before = readFileSync(METAINFO, "utf8");
    if (!/<releases>[\s\S]*<\/releases>/.test(before)) {
      console.error(`${METAINFO} has no <releases> block.`);
      process.exit(1);
    }
    const homepage = before.match(/<url type="homepage">([^<]+)<\/url>/)?.[1];
    writeFileSync(
      METAINFO,
      before.replace(/<releases>[\s\S]*<\/releases>/, toReleasesXml(releases, homepage)),
    );
    console.log(`Wrote ${releases.length} releases into the metainfo.`);
  }
}
