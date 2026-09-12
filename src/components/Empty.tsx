import { Icon } from "@/components/Icon";
import type { IconName } from "@/components/Icon";

/** Placeholder for a view with nothing to list yet. */
export function Empty({
  children,
  title,
  icon,
}: {
  children: React.ReactNode;
  title: string;
  icon: IconName;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-8 text-center">
      <Icon name={icon} className="h-10 w-10 text-foreground/20" />
      <h2 className="text-sm font-medium text-foreground/70">{title}</h2>
      <p className="max-w-sm text-xs text-foreground/45">{children}</p>
    </div>
  );
}
