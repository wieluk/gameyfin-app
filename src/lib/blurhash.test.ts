import { afterEach, describe, expect, it, vi } from "vitest";

import { blurhashBackground, blurhashUrl } from "./blurhash";

function fakeCanvas() {
  const putImageData = vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }),
    putImageData,
  } as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockReturnValue("data:image/png;base64,AAAA");
  return putImageData;
}

describe("blurhash", () => {
  afterEach(() => vi.restoreAllMocks());

  it("decodes a hash once into a data URL", () => {
    const put = fakeCanvas();
    expect(blurhashUrl("LEHV6nWB2yk8pyo0adR*.7kCMdnj")).toBe("data:image/png;base64,AAAA");
    expect(blurhashUrl("LEHV6nWB2yk8pyo0adR*.7kCMdnj")).toBe("data:image/png;base64,AAAA");
    expect(put).toHaveBeenCalledTimes(1);
  });

  it("paints it as a cover background", () => {
    fakeCanvas();
    expect(blurhashBackground("LKO2?U%2Tw=w]~RBVZRi};RPxuwH")).toEqual({
      backgroundImage: "url(data:image/png;base64,AAAA)",
      backgroundSize: "cover",
    });
  });

  it("gives nothing for a missing or malformed hash", () => {
    fakeCanvas();
    expect(blurhashUrl(null)).toBeNull();
    expect(blurhashUrl("not a hash")).toBeNull();
    expect(blurhashBackground(undefined)).toBeUndefined();
  });
});
