import { useQuery } from "@tanstack/react-query";

import { backend } from "@/lib/backend";

/** The library, with each game's local state. Invalidated as `["entries"]`. */
export function useEntries() {
  return useQuery({ queryKey: ["entries"], queryFn: () => backend.listEntries() });
}

/** Stored preferences. Invalidated as `["app-settings"]`. */
export function useAppSettings() {
  return useQuery({ queryKey: ["app-settings"], queryFn: () => backend.getSettings() });
}

/**
 * Whether a server is configured and signed in. Invalidated as `["status"]`.
 *
 * Never served from cache: the welcome wizard decides whether to show itself from this.
 *
 * Polled only while the server is unreachable, so the offline banner clears itself once
 * the network comes back rather than waiting for the user to do something. There is no
 * reason to keep asking while the answer is "connected".
 */
export function useStatus() {
  return useQuery({
    queryKey: ["status"],
    queryFn: () => backend.connectionStatus(),
    staleTime: 0,
    refetchInterval: (query) => (query.state.data?.offline ? 15_000 : false),
  });
}
