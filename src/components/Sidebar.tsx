import { useEffect, useState } from "react";
import { NavLink } from "react-router-dom";

import { readStored, writeStored } from "@/lib/storage";
import { Icon, type IconName } from "./Icon";

interface NavItem {
  to: string;
  label: string;
  icon: IconName;
  badge?: number;
}

const COLLAPSED_KEY = "gameyfin.sidebar.collapsed";

/**
 * Primary navigation. `h-full` keeps the panel the height of the window rather than the
 * current view, so it does not visibly resize between tabs.
 */
export function Sidebar({
  downloadCount,
  conflictCount,
}: {
  downloadCount: number;
  conflictCount: number;
}) {
  const [collapsed, setCollapsed] = useState(() => readStored(COLLAPSED_KEY, false));

  useEffect(() => writeStored(COLLAPSED_KEY, collapsed), [collapsed]);

  const items: NavItem[] = [
    { to: "/", label: "Library", icon: "library" },
    { to: "/downloads", label: "Downloads", icon: "download", badge: downloadCount || undefined },
    { to: "/installed", label: "Installed", icon: "installed" },
    { to: "/saves", label: "Saves", icon: "cloud", badge: conflictCount || undefined },
    { to: "/settings", label: "Settings", icon: "settings" },
  ];

  return (
    <nav
      className={`flex h-full shrink-0 flex-col gap-1 border-r-2 border-default-200 bg-content1 p-3 dark:border-default-200/60 transition-[width] duration-150 ${
        collapsed ? "w-[60px]" : "w-52"
      }`}
    >
      {items.map((item) => (
        <NavLink
          key={item.to}
          to={item.to}
          end={item.to === "/"}
          title={collapsed ? item.label : undefined}
          className={({ isActive }) =>
            `flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors ${
              collapsed ? "justify-center" : ""
            } ${
              isActive
                ? "bg-primary/15 font-medium text-primary"
                : "text-foreground/70 hover:bg-default-100 hover:text-foreground"
            }`
          }
        >
          <span className="relative shrink-0">
            <Icon name={item.icon} className="h-[18px] w-[18px]" />
            {/* Collapsed, the badge becomes a dot on the icon; the count reads in the tooltip. */}
            {collapsed && item.badge ? (
              <span className="absolute -right-1 -top-1 h-2 w-2 rounded-full bg-primary ring-2 ring-content1" />
            ) : null}
          </span>

          {!collapsed && (
            <>
              <span className="flex-1 truncate">{item.label}</span>
              {item.badge ? (
                <span className="rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-semibold text-white">
                  {item.badge}
                </span>
              ) : null}
            </>
          )}
        </NavLink>
      ))}

      <button
        type="button"
        onClick={() => setCollapsed((current: boolean) => !current)}
        aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        title={collapsed ? "Expand" : "Collapse"}
        className={`mt-auto flex items-center gap-3 rounded-lg px-3 py-2 text-sm text-foreground/45 transition-colors hover:bg-default-100 hover:text-foreground ${
          collapsed ? "justify-center" : ""
        }`}
      >
        <Icon
          name="chevron"
          className={`h-[18px] w-[18px] shrink-0 transition-transform ${
            collapsed ? "" : "rotate-180"
          }`}
        />
        {!collapsed && <span className="truncate">Collapse</span>}
      </button>
    </nav>
  );
}
