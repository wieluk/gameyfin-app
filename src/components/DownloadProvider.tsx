import { useQuery, useQueryClient } from "@tanstack/react-query";

import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/**
 * Which of the server's providers a download comes from.
 *
 * Hidden unless the server offers a choice: one provider is the common case, and a
 * dropdown with a single entry is only clutter.
 */
export function DownloadProvider({ onError }: { onError: (message: string | null) => void }) {
  const queryClient = useQueryClient();

  const providers = useQuery({
    queryKey: ["download-providers"],
    queryFn: () => backend.downloadProviders(),
    // Plugins are enabled server-side, rarely and by an admin.
    staleTime: 5 * 60 * 1000,
    retry: false,
  });

  const options = providers.data ?? [];
  if (options.length < 2) return null;

  const selected = options.find((p) => p.selected) ?? options[0];

  async function change(key: string) {
    onError(null);
    try {
      await backend.setDownloadProvider(key);
      await queryClient.invalidateQueries({ queryKey: ["download-providers"] });
    } catch (e) {
      onError(messageOf(e));
    }
  }

  return (
    <div className="flex items-center gap-2">
      <label className="text-xs text-foreground/45" htmlFor="download-provider">
        From
      </label>
      <select
        id="download-provider"
        value={selected.key}
        title={selected.description}
        onChange={(e) => void change(e.target.value)}
        className="rounded-lg border border-default-200 bg-content2 px-2.5 py-1.5 text-xs outline-none focus:border-primary"
      >
        {options.map((provider) => (
          <option key={provider.key} value={provider.key}>
            {provider.name}
          </option>
        ))}
      </select>
      {selected.needsTorrentClient && (
        <span className="text-[11px] text-warning-600">
          Gives a .torrent, which Gameyfin cannot download from yet.
        </span>
      )}
    </div>
  );
}
