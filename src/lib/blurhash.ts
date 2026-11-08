import { decode } from "blurhash";

const SIZE = 32;
const cache = new Map<string, string | null>();

/** A blurhash as a tiny data URL, decoded once: the placeholder is a CSS background, not a canvas per card. */
export function blurhashUrl(hash: string | null | undefined): string | null {
  if (!hash) return null;
  const cached = cache.get(hash);
  if (cached !== undefined) return cached;
  let url: string | null = null;
  try {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = SIZE;
    const context = canvas.getContext("2d");
    if (context) {
      const pixels = context.createImageData(SIZE, SIZE);
      pixels.data.set(decode(hash, SIZE, SIZE));
      context.putImageData(pixels, 0, 0);
      url = canvas.toDataURL();
    }
  } catch {
    // A malformed hash only costs the placeholder.
  }
  cache.set(hash, url);
  return url;
}

/** Inline style that paints the placeholder behind an image until it loads. */
export function blurhashBackground(hash: string | null | undefined) {
  const url = blurhashUrl(hash);
  return url ? { backgroundImage: `url(${url})`, backgroundSize: "cover" } : undefined;
}
