import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";

import type { AppNotification } from "@/bindings/AppNotification";
import { Icon } from "@/components/Icon";
import { Button, IconButton } from "@/components/ui";
import { backend } from "@/lib/backend";
import { formatRelative } from "@/lib/format";
import { keys } from "@/lib/queries";
import { useDismissOnEscape } from "@/lib/useDismiss";
import { useTauriEvent } from "@/lib/useTauriEvent";

const TONE: Record<AppNotification["category"], string> = {
  transfer: "bg-primary",
  action: "bg-warning",
  failure: "bg-danger",
  update: "bg-success",
};

/** The bell in the title bar: every notification this session, whether or not it popped up. */
export function NotificationBell() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const notifications = useQuery({
    queryKey: keys.notifications,
    queryFn: () => backend.listNotifications(),
  });
  useTauriEvent("notifications-changed", () => {
    void queryClient.invalidateQueries({ queryKey: keys.notifications });
  });

  const items = notifications.data ?? [];
  const unread = items.filter((n) => !n.read).length;

  function toggle() {
    const next = !open;
    setOpen(next);
    // Opening the list is reading it.
    if (next && unread > 0) void backend.markNotificationsRead();
  }

  return (
    <div className="relative">
      <button
        type="button"
        aria-label={unread > 0 ? `Notifications, ${unread} unread` : "Notifications"}
        aria-expanded={open}
        onClick={toggle}
        className={`relative flex h-6 w-8 items-center justify-center rounded transition-colors hover:bg-default-200 hover:text-foreground ${
          open ? "bg-default-200 text-foreground" : "text-foreground/60"
        }`}
      >
        <Icon name="bell" className="h-3.5 w-3.5" />
        {unread > 0 && (
          <span className="absolute right-0.5 top-0 h-3.5 min-w-[14px] rounded-full bg-primary px-1 text-center text-[9px] font-semibold leading-[14px] text-white">
            {unread > 9 ? "9+" : unread}
          </span>
        )}
      </button>
      {open && <NotificationPanel items={items} onClose={() => setOpen(false)} />}
    </div>
  );
}

function NotificationPanel({
  items,
  onClose,
}: {
  items: AppNotification[];
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const panel = useRef<HTMLDivElement>(null);
  useDismissOnEscape(onClose);

  // A click anywhere else closes it, as a menu would. The bell toggles it itself.
  useEffect(() => {
    function outside(event: MouseEvent) {
      const holder = panel.current?.parentElement;
      if (holder && !holder.contains(event.target as Node)) onClose();
    }
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, [onClose]);

  function openItem(item: AppNotification) {
    onClose();
    if (item.route) navigate(item.route);
    void backend.dismissNotification(item.id);
  }

  return (
    <div
      ref={panel}
      role="dialog"
      aria-label="Notifications"
      data-nav-scope
      className="absolute right-0 top-8 z-[65] w-80 overflow-hidden rounded-xl border border-default-200 bg-content1 shadow-2xl"
    >
      <div className="flex items-center justify-between border-b border-default-200/60 px-3 py-2">
        <h2 className="text-xs font-semibold text-foreground">Notifications</h2>
        {items.length > 0 && (
          <Button size="sm" variant="ghost" onClick={() => void backend.dismissNotification(null)}>
            Dismiss all
          </Button>
        )}
      </div>

      {items.length === 0 ? (
        <p className="px-3 py-6 text-center text-xs text-foreground/45">Nothing new.</p>
      ) : (
        <ul className="max-h-96 overflow-y-auto">
          {items.map((item) => (
            <li
              key={item.id}
              className="flex items-start border-b border-default-200/40 last:border-0"
            >
              <button
                type="button"
                onClick={() => openItem(item)}
                className="flex min-w-0 flex-1 items-start gap-2.5 px-3 py-2.5 text-left transition-colors hover:bg-default-100"
              >
                <span
                  className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${TONE[item.category]} ${
                    item.read ? "opacity-40" : ""
                  }`}
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs font-medium text-foreground">
                    {item.title}
                  </span>
                  <span className="mt-0.5 line-clamp-2 block text-[11px] leading-snug text-foreground/60">
                    {item.body}
                  </span>
                  <span className="mt-1 block text-[10px] text-foreground/40">
                    {formatRelative(item.createdAt)}
                  </span>
                </span>
              </button>
              <IconButton
                icon="close"
                size="sm"
                label={`Dismiss ${item.title}`}
                className="mr-1.5 mt-2"
                onClick={() => void backend.dismissNotification(item.id)}
              />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
