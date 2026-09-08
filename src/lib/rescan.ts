import { useEffect } from "react";

import { backend } from "./backend";

/**
 * Adopt whatever is on disk when a view opens.
 *
 * Opening Library, Downloads or Installed is the moment the user expects to see what is
 * actually there, so anything added, moved or deleted outside the app is picked up
 * without them having to ask. Failures are swallowed on purpose: this runs unbidden, and
 * a rescan that could not read the folder is not something to interrupt the view with.
 * The Rescan button is the version that reports what went wrong.
 *
 * The backend emits `library-changed` when the scan finds something, which is what
 * refreshes the list, so nothing is invalidated here.
 */
export function useRescanOnOpen() {
  useEffect(() => {
    void backend.rescanLibrary().catch(() => {});
    // Deliberately once per mount rather than on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
