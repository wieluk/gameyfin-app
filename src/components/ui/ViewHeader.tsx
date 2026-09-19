import { Alert } from "@/components/Alert";
import { HelpButton } from "@/components/HelpButton";
import type { HelpTopic } from "@/lib/help";

/** The bar across the top of a tab, so every tab opens the same way. */
export function ViewHeader({
  title,
  actions,
  error,
  help,
  children,
}: {
  title?: string;
  actions?: React.ReactNode;
  /** Travels with the header, since its actions can fail while the list is empty. */
  error?: string | null;
  /** The docs section about this tab, opened by a help button at the end of the bar. */
  help?: HelpTopic;
  /** A toolbar in place of a title, as in Library. */
  children?: React.ReactNode;
}) {
  return (
    <>
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b-2 border-default-200 px-6 py-3 dark:border-default-200/60">
        {title && (
          <h2 className="text-xs font-semibold uppercase tracking-wide text-foreground/45">
            {title}
          </h2>
        )}
        {actions && <div className="flex flex-wrap items-center justify-end gap-2">{actions}</div>}
        {children}
        {help && <HelpButton topic={help} />}
      </div>
      {error && <Alert className="mx-6 mt-3">{error}</Alert>}
    </>
  );
}
