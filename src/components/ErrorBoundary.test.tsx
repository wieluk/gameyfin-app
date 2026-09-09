import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ErrorBoundary } from "./ErrorBoundary";

function Boom(): never {
  throw new Error("latest is an object");
}

describe("ErrorBoundary", () => {
  beforeEach(() => {
    // React logs the caught error itself; the test asserts behaviour, not the noise.
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => vi.restoreAllMocks());

  it("shows a way out instead of a blank window", () => {
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    );

    expect(screen.getByRole("button", { name: "Reload" })).toBeTruthy();
    // The message is what makes a support report possible at all.
    expect(screen.getByText("latest is an object")).toBeTruthy();
  });

  it("stays out of the way when nothing throws", () => {
    render(
      <ErrorBoundary>
        <p>the library</p>
      </ErrorBoundary>,
    );

    expect(screen.getByText("the library")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Reload" })).toBeNull();
  });
});
