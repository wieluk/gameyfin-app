import { useEffect } from "react";

import { backend } from "./backend";

/**
 * Adopt whatever is on disk when a view opens. Failures are swallowed, since the Rescan button
 * reports them, and the backend emits `library-changed` when it finds something.
 */
export function useRescanOnOpen() {
  useEffect(() => {
    void backend.rescanLibrary().catch(() => {});
    // Deliberately once per mount rather than on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
