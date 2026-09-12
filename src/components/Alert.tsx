/** An error, styled the same everywhere it appears. `inline` drops the box. */
export function Alert({
  children,
  inline = false,
  className = "",
}: {
  children: React.ReactNode;
  inline?: boolean;
  className?: string;
}) {
  const style = inline
    ? "text-[11px] leading-relaxed text-danger"
    : "rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger";
  return (
    <p role="alert" className={`${style} ${className}`}>
      {children}
    </p>
  );
}
