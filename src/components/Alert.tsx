/** An inline error, styled the same everywhere it appears. */
export function Alert({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return (
    <p
      role="alert"
      className={`rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger ${className}`}
    >
      {children}
    </p>
  );
}
