/** Query keys and the hooks that read them, so an invalidation cannot name a key wrongly. */

import { useQuery, useQueryClient } from "@tanstack/react-query";

import { backend } from "@/lib/backend";
import type { SettingsPatch } from "@/types";

export const keys = {
  entries: ["entries"] as const,
  libraries: ["libraries"] as const,
  status: ["status"] as const,
  settings: ["app-settings"] as const,
  libraryRoots: ["library-roots"] as const,
  installPlan: (gameId: number) => ["install-plan", gameId] as const,
  untracked: ["untracked-folders"] as const,
  notifications: ["notifications"] as const,
  /** Every game's plan or options, for a change that affects all of them. */
  installPlans: ["install-plan"] as const,
  gameOptions: (gameId: number) => ["game-options", gameId] as const,
  gameOptionsAll: ["game-options"] as const,
  /** Every game's save status for one scope; `saveOverviewAll` invalidates both scopes. */
  saveOverview: (scope: string) => ["save-overview", scope] as const,
  saveOverviewAll: ["save-overview"] as const,
  saveVersions: (gameId: number) => ["save-versions", gameId] as const,
  saveTool: ["save-tool-status"] as const,
  proton: ["proton-status"] as const,
  umu: ["umu-status"] as const,
  prefixes: ["prefix-list"] as const,
  update: ["update-status"] as const,
  memory: ["memory-info"] as const,
  diagnostics: ["diagnostics"] as const,
};

/** Invalidate one or more keys, for a change that touches several screens. */
export function useInvalidate() {
  const client = useQueryClient();
  return (...queryKeys: ReadonlyArray<readonly unknown[]>) =>
    Promise.all(queryKeys.map((queryKey) => client.invalidateQueries({ queryKey })));
}

/** The library, with each game's local state. */
export function useEntries() {
  return useQuery({ queryKey: keys.entries, queryFn: () => backend.listEntries() });
}

export function useAppSettings() {
  return useQuery({ queryKey: keys.settings, queryFn: () => backend.getSettings() });
}

/**
 * Change some settings and pick up the result. The patch says what changed, so two
 * controls toggled in quick succession cannot write each other's stale value back.
 */
export function useSettingsUpdate() {
  const settings = useAppSettings();
  const invalidate = useInvalidate();
  return async (patch: SettingsPatch) => {
    await backend.updateSettings(patch);
    await settings.refetch();
    // A different save location makes every game's save state stale.
    if (patch.saveBackend || patch.saveFolder || patch.webdavUrl || patch.saveSyncEnabled !== undefined) {
      await invalidate(keys.saveOverviewAll);
    }
  };
}

/**
 * Whether a server is configured and signed in. Never cached, and polled while unreachable or
 * signed out, so the offline banner clears and a misread failure cannot strand the user.
 */
export function useStatus() {
  return useQuery({
    queryKey: keys.status,
    queryFn: () => backend.connectionStatus(),
    staleTime: 0,
    refetchInterval: (query) => {
      const status = query.state.data;
      if (!status) return false;
      return status.offline || (status.configured && !status.authenticated) ? 15_000 : false;
    },
  });
}

export function useProtonStatus() {
  return useQuery({ queryKey: keys.proton, queryFn: () => backend.protonStatus() });
}

export function useSaveToolStatus() {
  return useQuery({ queryKey: keys.saveTool, queryFn: () => backend.saveToolStatus() });
}
