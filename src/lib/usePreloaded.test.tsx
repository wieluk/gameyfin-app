import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { usePreloaded } from "./usePreloaded";

/** A promise resolved from the test, so the order of answers is chosen, not raced. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

describe("usePreloaded", () => {
  it("opens only once the data is in", async () => {
    const answer = deferred<string>();
    const { result } = renderHook(() => usePreloaded((_id: number) => answer.promise));

    let opening!: Promise<void>;
    act(() => {
      opening = result.current.open(7);
    });
    expect(result.current.loading).toBe(7);
    expect(result.current.opened).toBeNull();

    await act(async () => {
      answer.resolve("plan");
      await opening;
    });
    expect(result.current.opened).toEqual({ target: 7, data: "plan" });
    expect(result.current.loading).toBeNull();
  });

  it("lets the latest request win over a slower earlier one", async () => {
    const answers = new Map([
      [1, deferred<string>()],
      [2, deferred<string>()],
    ]);
    const { result } = renderHook(() =>
      usePreloaded((id: number) => answers.get(id)!.promise),
    );

    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => {
      first = result.current.open(1);
      second = result.current.open(2);
    });
    await act(async () => {
      answers.get(2)!.resolve("two");
      await second;
      answers.get(1)!.resolve("one");
      await first;
    });
    expect(result.current.opened).toEqual({ target: 2, data: "two" });
  });

  it("hands a failure to the caller and opens nothing", async () => {
    const { result } = renderHook(() =>
      usePreloaded(async (_id: number) => {
        throw new Error("no plan");
      }),
    );
    await act(async () => {
      await expect(result.current.open(3)).rejects.toThrow("no plan");
    });
    expect(result.current.opened).toBeNull();
    expect(result.current.loading).toBeNull();
  });

  it("opens nothing that was cancelled while it loaded", async () => {
    const answer = deferred<string>();
    const { result } = renderHook(() => usePreloaded((_id: number) => answer.promise));
    let opening!: Promise<void>;
    act(() => {
      opening = result.current.open(4);
    });
    act(() => result.current.close());
    await act(async () => {
      answer.resolve("late");
      await opening;
    });
    expect(result.current.opened).toBeNull();
  });
});
