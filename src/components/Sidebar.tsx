import { useEffect, useState } from "react";
import { NavLink } from "react-router-dom";
import { Icon, type IconName } from "./Icon";

interface NavItem {
  to: string;
  label: string;
  icon: IconName;
  badge?: number;
}

const COLLAPSED_KEY = "gameyfin.sidebar.collapsed";

/**
 * Primary navigation.
 *
 * `h-full` keeps it the height of the window rather than the height of whatever the
 * current view happens to contain, otherwise the panel visibly resizes as you move
 * between a full library and an empty Downloads tab.
 */
export function Sidebar({ downloadCount }: { downloadCount: number }) {
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem(COLLAPSED_KEY) === "1";
    } catch {
      // Private windows and blocked site data both throw here; the default is fine.
      return false;
    }
  });

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
    } catch {
      // A remembered preference is a convenience, not a requirement.
    }
  }, [collapsed]);

  const items: NavItem[] = [
    { to: "/", label: "Library", icon: "library" },
    { to: "/downloads", label: "Downloads", icon: "download", badge: downloadCount || undefined },
    { to: "/installed", label: "Installed", icon: "installed" },
    { to: "/settings", label: "Settings", icon: "settings" },
  ];

  return (
    <nav
      className={`flex h-full shrink-0 flex-col gap-1 border-r border-default-200/60 bg-content1 p-3 transition-[width] duration-150 ${
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
            {/* Collapsed, there is no room for a badge beside the label, so it becomes a
                dot on the icon, the count still reads in the tooltip. */}
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
        onClick={() => setCollapsed((c) => !c)}
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
