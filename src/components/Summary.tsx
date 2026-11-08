import type { ComponentProps, ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import { backend } from "@/lib/backend";

/** Before 2.6.1 Gameyfin stored summaries as HTML, so tags and entities become plain text first. */
export function legacyHtmlToText(summary: string): string {
  if (!/<\/?[a-z][^>]*>|&[#a-z0-9]+;/i.test(summary)) return summary;
  const doc = new DOMParser().parseFromString(summary, "text/html");
  doc.querySelectorAll("br").forEach((br) => br.replaceWith("\n"));
  doc.querySelectorAll("p, div, li, h1, h2, h3, h4, h5, h6").forEach((block) => block.append("\n\n"));
  return (doc.body.textContent ?? "").replace(/\n{3,}/g, "\n\n").trim();
}

function Heading({ children }: { children?: ReactNode }) {
  return <p className="font-semibold text-foreground/85">{children}</p>;
}

const components: Components = {
  p: ({ children }) => <p>{children}</p>,
  h1: Heading,
  h2: Heading,
  h3: Heading,
  h4: Heading,
  h5: Heading,
  h6: Heading,
  ul: ({ children }) => <ul className="list-disc space-y-1 pl-5">{children}</ul>,
  ol: ({ children }) => <ol className="list-decimal space-y-1 pl-5">{children}</ol>,
  strong: ({ children }) => <strong className="font-semibold text-foreground/85">{children}</strong>,
  code: ({ children }) => <code className="font-mono text-[0.8125rem]">{children}</code>,
  blockquote: ({ children }) => (
    <blockquote className="border-l-2 border-foreground/15 pl-3">{children}</blockquote>
  ),
  a: ({ href, children }: ComponentProps<"a">) => (
    <a
      href={href}
      className="underline underline-offset-2 hover:text-foreground"
      onClick={(event) => {
        event.preventDefault();
        // The backend opens only http and https in the browser.
        if (href) void backend.openUrl(href);
      }}
    >
      {children}
    </a>
  ),
};

/** A game summary, Markdown since Gameyfin 2.6.1. Raw HTML and images are dropped. */
export function Summary({ text, className = "" }: { text: string; className?: string }) {
  return (
    <div className={`min-w-0 space-y-3 break-words ${className}`}>
      <Markdown
        remarkPlugins={[remarkBreaks]}
        components={components}
        disallowedElements={["img"]}
        skipHtml
      >
        {legacyHtmlToText(text)}
      </Markdown>
    </div>
  );
}
