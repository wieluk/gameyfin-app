import { describe, expect, it } from "vitest";

import { videoFile, youtubeId } from "./Trailers";

describe("youtubeId", () => {
  it("reads the id from every form the metadata uses", () => {
    expect(youtubeId("https://www.youtube.com/watch?v=dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
    expect(youtubeId("https://youtu.be/dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
    expect(youtubeId("https://www.youtube.com/embed/dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
    expect(youtubeId("https://m.youtube.com/watch?v=dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
    expect(youtubeId("https://www.youtube.com/shorts/dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
  });

  it("keeps other query parameters out of the id", () => {
    expect(youtubeId("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s")).toBe("dQw4w9WgXcQ");
  });

  it("rejects a lookalike host", () => {
    // Parsed as a URL rather than matched with a pattern precisely so that this cannot
    // become an embed pointing at somebody else's server.
    expect(youtubeId("https://youtube.com.evil.example/watch?v=abc123")).toBeNull();
    expect(youtubeId("https://notyoutube.com/watch?v=abc123")).toBeNull();
  });

  it("rejects anything that is not a usable link", () => {
    expect(youtubeId("")).toBeNull();
    expect(youtubeId("not a url")).toBeNull();
    expect(youtubeId("https://vimeo.com/12345")).toBeNull();
    // A non-web scheme has no business in a frame.
    expect(youtubeId("javascript:alert(1)")).toBeNull();
    expect(youtubeId("file:///etc/passwd")).toBeNull();
  });

  it("rejects an id outside the expected alphabet", () => {
    // Anything else would be pasted straight into the embed URL.
    expect(youtubeId("https://www.youtube.com/watch?v=../../evil")).toBeNull();
    expect(youtubeId("https://youtu.be/short")).toBeNull();
  });
});

describe("videoFile", () => {
  it("recognises the trailers the Steam plugin hands over", () => {
    // What that plugin puts in `videoUrls`: the file itself, not a page about it.
    expect(videoFile("https://video.akamai.steamstatic.com/store_trailers/1/movie_max.webm")).toBe(
      true,
    );
    expect(videoFile("https://cdn.example.com/trailer.mp4")).toBe(true);
    expect(videoFile("https://cdn.example.com/trailer.MP4")).toBe(true);
  });

  it("leaves anything that is not a playable file to the browser", () => {
    expect(videoFile("https://www.youtube.com/watch?v=dQw4w9WgXcQ")).toBe(false);
    expect(videoFile("https://vimeo.com/12345")).toBe(false);
    expect(videoFile("not a url")).toBe(false);
    // Plain HTTP and local files have no business in the player.
    expect(videoFile("http://cdn.example.com/trailer.mp4")).toBe(false);
    expect(videoFile("file:///home/ana/trailer.mp4")).toBe(false);
  });
});
