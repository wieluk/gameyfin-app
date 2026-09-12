import { describe, expect, it } from "vitest";

import { signingProblems } from "./check-signing.mjs";

/** A key file as `tauri signer generate` writes it: minisign text, base64-wrapped. */
const encode = (text) => Buffer.from(text, "utf8").toString("base64");
const KEY = encode("untrusted comment: rsign encrypted secret key\nRWRTY0Iy...\n");
const PUB = encode("untrusted comment: minisign public key: ABC\nRWTdP792...\n");

const config = (pubkey, createUpdaterArtifacts = true) => ({
  bundle: { createUpdaterArtifacts },
  plugins: { updater: { pubkey } },
});

describe("signingProblems", () => {
  it("passes when both halves are present", () => {
    expect(signingProblems({ privateKey: KEY, config: config(PUB) })).toEqual([]);
  });

  it("catches the empty secret that failed the release", () => {
    // GitHub substitutes an empty string for a secret that does not exist, and Tauri
    // reports that only as "Missing comment in secret key" after the whole build.
    const problems = signingProblems({ privateKey: "", config: config(PUB) });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("TAURI_SIGNING_PRIVATE_KEY");
  });

  it("catches an unset secret", () => {
    expect(signingProblems({ privateKey: undefined, config: config(PUB) })).toHaveLength(1);
  });

  it("catches a key pasted decoded rather than as the file's contents", () => {
    const raw = "untrusted comment: rsign encrypted secret key\nRWRTY0Iy...\n";
    const problems = signingProblems({ privateKey: raw, config: config(PUB) });
    expect(problems[0]).toContain("does not look like a Tauri key file");
  });

  it("catches the key pasted twice, which the release failed on", () => {
    // Padding mid-value: Node's decoder skips it, so the key would fail only after the build.
    const [problem] = signingProblems({ privateKey: KEY + KEY, config: config(PUB) });
    expect(problem).toContain("something after a complete key");
  });

  it("catches base64 appended to a good key", () => {
    const [problem] = signingProblems({ privateKey: `${KEY}AAAA`, config: config(PUB) });
    expect(problem).toContain("something after a complete key");
  });

  it("accepts the trailing newline a secret store adds", () => {
    expect(signingProblems({ privateKey: `${KEY}\n`, config: config(PUB) })).toEqual([]);
  });

  it("catches the two halves swapped", () => {
    const problems = signingProblems({ privateKey: PUB, config: config(KEY) });
    expect(problems[0]).toContain("holds a public key");
    // And says plainly why shipping the private half is worse than a broken build.
    expect(problems[1]).toContain("sign an update your app accepts");
  });

  it("catches an empty pubkey, which breaks updates without failing the build", () => {
    const problems = signingProblems({ privateKey: KEY, config: config("") });
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("pubkey");
  });

  it("reports both halves at once", () => {
    expect(signingProblems({ privateKey: "", config: config("") })).toHaveLength(2);
  });

  it("checks nothing when the release ships no updater artifacts", () => {
    expect(signingProblems({ privateKey: "", config: config("", false) })).toEqual([]);
  });
});
