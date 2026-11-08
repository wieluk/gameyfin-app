import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { legacyHtmlToText, Summary } from "./Summary";

const openUrl = vi.fn(async () => {});
vi.mock("@/lib/backend", () => ({
  backend: { openUrl: (...args: unknown[]) => openUrl(...(args as [])) },
}));

describe("Summary", () => {
  it("renders Markdown from Gameyfin 2.6.1", () => {
    const { container } = render(
      <Summary text={"A **bold** climb.\nNext line\n\n- one\n- two"} />,
    );
    expect(container.querySelector("strong")?.textContent).toBe("bold");
    expect(container.querySelectorAll("li")).toHaveLength(2);
    expect(container.querySelector("br")).not.toBeNull();
    expect(container.textContent).not.toContain("**");
  });

  it("opens links in the browser", () => {
    render(<Summary text="See [the site](https://example.com)." />);
    fireEvent.click(screen.getByText("the site"));
    expect(openUrl).toHaveBeenCalledWith("https://example.com");
  });

  it("drops images and raw HTML", () => {
    const { container } = render(
      <Summary text={"![x](https://example.com/a.png)\n\nText <script>alert(1)</script>"} />,
    );
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("script")).toBeNull();
  });

  it("reads an HTML summary from an older server as text", () => {
    expect(legacyHtmlToText("<p>Tom &amp; Jerry</p><p>One<br>Two</p>")).toBe(
      "Tom & Jerry\n\nOne\nTwo",
    );
    expect(legacyHtmlToText("Plain *text*")).toBe("Plain *text*");
  });
});
