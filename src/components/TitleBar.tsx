import { isMockBackend } from "@/lib/backend";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { GamepadIndicator } from "./GamepadOverlay";
import { Icon } from "./Icon";

/**
 * Custom window chrome. Tauri moves the window via `data-tauri-drag-region`; the CSS
 * `-webkit-app-region` property used by Electron has no effect here.
 */
export function TitleBar() {
  return (
    <header
      data-tauri-drag-region
      onDoubleClick={() => void windowAction("toggleMaximize")}
      className="flex h-9 shrink-0 select-none items-center justify-between border-b-2 border-default-200 bg-content1 px-3 dark:border-default-200/60"
    >
      {/* Children of a drag region are not draggable, so repeat the attribute. */}
      <div data-tauri-drag-region className="pointer-events-none flex items-center gap-2">
        <Icon name="controller" className="h-4 w-4 text-primary" />
        <span className="text-xs font-medium tracking-wide text-foreground/70">
          Gameyfin
        </span>
      </div>

      <div className="flex items-center gap-1">
        <GamepadIndicator />
        <WindowButton label="Minimise" onClick={() => void windowAction("minimize")}>
          <span className="block h-px w-2.5 bg-current" />
        </WindowButton>
        <WindowButton label="Maximise" onClick={() => void windowAction("toggleMaximize")}>
          <span className="block h-2 w-2 border border-current" />
        </WindowButton>
        <WindowButton label="Close" danger onClick={() => void windowAction("close")}>
          <Icon name="close" className="h-3 w-3" />
        </WindowButton>
      </div>
    </header>
  );
}

type Action = "minimize" | "toggleMaximize" | "close";

async function windowAction(action: Action) {
  if (isMockBackend) return;
  const w = getCurrentWindow();
  if (action === "minimize") await w.minimize();
  else if (action === "toggleMaximize") await w.toggleMaximize();
  else await w.close();
}

function WindowButton({
  children,
  label,
  onClick,
  danger,
}: {
  children: React.ReactNode;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className={`flex h-6 w-8 items-center justify-center rounded text-foreground/60 transition-colors hover:text-foreground ${
        danger ? "hover:bg-danger hover:text-white" : "hover:bg-default-200"
      }`}
    >
      {children}
    </button>
  );
}
