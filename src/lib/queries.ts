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
 * Whether a server is configured and signed in. Invalidated as `["status"]`. Never served
 * from cache (the welcome wizard reads it), and polled while the server is unreachable so
 * the offline banner clears itself.
 *
 * Also polled when a server is configured but not signed in: that state is either a real
 * sign-out, which the poll costs nothing, or a misread connection failure, which without
 * the poll would strand the user in the wizard until they restarted the app.
 */
export function useStatus() {
  return useQuery({
    queryKey: ["status"],
    queryFn: () => backend.connectionStatus(),
    staleTime: 0,
    refetchInterval: (query) => {
      const status = query.state.data;
      if (!status) return false;
      return status.offline || (status.configured && !status.authenticated) ? 15_000 : false;
    },
  });
}
