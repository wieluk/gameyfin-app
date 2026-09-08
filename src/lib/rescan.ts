import { useEffect } from "react";

import { backend } from "./backend";

/**
 * Adopt whatever is on disk when a view opens, so changes made outside the app show up.
 * Failures are swallowed on purpose (the Rescan button is the one that reports them); the
 * backend emits `library-changed` when it finds something, so nothing is invalidated here.
 */
export function useRescanOnOpen() {
  useEffect(() => {
    void backend.rescanLibrary().catch(() => {});
    // Deliberately once per mount rather than on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
