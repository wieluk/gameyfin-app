import { useState } from "react";
import { isWindows } from "@/lib/platform";
import { readStored, writeStored } from "@/lib/storage";
import { PANEL_BODY } from "@/lib/ui";
import { AccountSection } from "./settings/AccountSection";
import { AboutSection } from "./settings/AboutSection";
import {
  CompatibilitySection,
  PrefixSection,
  ProtonSection,
  UmuSection,
} from "./settings/compatibility";
import { DiagnosticsSection } from "./settings/diagnostics";
import { AppearanceSection, GamepadSection, NotificationSection, WindowSection } from "./settings/interface";
import { DownloadSection, ExtractionSection, RootsSection } from "./settings/library";
import { MigrationSection, SaveToolSection, SavesSection } from "./settings/saves";

/** Which pane of Settings is showing. */
type TabId =
  | "account"
  | "library"
  | "interface"
  | "compatibility"
  | "saves"
  | "diagnostics"
  | "about";

const TAB_KEY = "gameyfin.settings.tab";

/** The panes, in order; a list so adding one is a single entry plus a branch below. */
const TABS: Array<{ id: TabId; label: string; hideOnWindows?: boolean }> = [
  { id: "account", label: "Account" },
  { id: "library", label: "Library" },
  { id: "interface", label: "Interface" },
  // Nothing to configure here on Windows, which runs its own programs.
  { id: "compatibility", label: "Compatibility", hideOnWindows: true },
  { id: "saves", label: "Saves" },
  { id: "diagnostics", label: "Diagnostics" },
  { id: "about", label: "About" },
];

export function SettingsView({ onSignedOut }: { onSignedOut: () => void }) {
  const tabs = TABS.filter((tab) => !(isWindows && tab.hideOnWindows));

  const [tab, setTab] = useState<TabId>(() => {
    const stored = readStored<TabId>(TAB_KEY, "account");
    // A stale or platform-hidden pane must not leave the view blank.
    return tabs.some((t) => t.id === stored) ? stored : "account";
  });

  function select(next: TabId) {
    setTab(next);
    writeStored(TAB_KEY, next);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        role="tablist"
        aria-label="Settings"
        className="flex shrink-0 items-center gap-1 border-b-2 border-default-200 px-6 dark:border-default-200/60"
      >
        {tabs.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            onClick={() => select(item.id)}
            className={`-mb-0.5 border-b-2 px-3 py-2.5 text-xs font-medium transition-colors ${
              tab === item.id
                ? "border-primary text-primary"
                : "border-transparent text-foreground/50 hover:text-foreground"
            }`}
          >
            {item.label}
          </button>
        ))}
      </div>

      <div className={PANEL_BODY}>
        <div className="mx-auto flex max-w-2xl flex-col gap-5">
          {tab === "account" && <AccountSection onSignedOut={onSignedOut} />}
          {tab === "library" && (
            <>
              <RootsSection />
              <DownloadSection />
              <ExtractionSection />
            </>
          )}
          {tab === "interface" && (
            <>
              <AppearanceSection />
              <NotificationSection />
              <WindowSection />
              <GamepadSection />
            </>
          )}
          {tab === "compatibility" && (
            <>
              <ProtonSection />
              <UmuSection />
              <CompatibilitySection />
              <PrefixSection />
            </>
          )}
          {tab === "saves" && (
            <>
              <SavesSection />
              <MigrationSection />
              <SaveToolSection />
            </>
          )}
          {tab === "diagnostics" && <DiagnosticsSection />}
          {tab === "about" && <AboutSection />}
        </div>
      </div>
    </div>
  );
}
