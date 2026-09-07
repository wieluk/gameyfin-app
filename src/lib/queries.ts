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
 */
export function useStatus() {
  return useQuery({
    queryKey: ["status"],
    queryFn: () => backend.connectionStatus(),
    staleTime: 0,
  });
}
