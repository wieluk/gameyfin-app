import { Alert } from "@/components/Alert";

/** The bar across the top of a tab, so every tab opens the same way. */
export function ViewHeader({
  title,
  actions,
  error,
  children,
}: {
  title?: string;
  actions?: React.ReactNode;
  /** Travels with the header, since its actions can fail while the list is empty. */
  error?: string | null;
  /** A toolbar in place of a title, as in Library. */
  children?: React.ReactNode;
}) {
  return (
    <>
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-default-200/60 px-6 py-3">
        {title && (
          <h2 className="text-xs font-semibold uppercase tracking-wide text-foreground/45">
            {title}
          </h2>
        )}
        {actions && <div className="flex flex-wrap items-center justify-end gap-2">{actions}</div>}
        {children}
      </div>
      {error && <Alert className="mx-6 mt-3">{error}</Alert>}
    </>
  );
}
