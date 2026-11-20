import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

/**
 * Agent answers are Markdown. Rendered with the same type scale as the rest of the
 * app; raw HTML in the source is not interpreted, so untrusted output stays inert.
 */
export function Markdown({
  children,
  className,
  components,
}: {
  children: string;
  className?: string;
  /** Overrides for individual elements, merged over the defaults (used by the docs reader). */
  components?: Components;
}) {
  return (
    <div className={cn("min-w-0 break-words text-sm leading-relaxed", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          h1: (props) => <h3 className="mb-2 mt-5 text-[17px] font-semibold first:mt-0" {...props} />,
          h2: (props) => <h3 className="mb-2 mt-5 text-base font-semibold first:mt-0" {...props} />,
          h3: (props) => <h4 className="mb-1.5 mt-4 text-[15px] font-semibold first:mt-0" {...props} />,
          h4: (props) => <h5 className="mb-1 mt-3 text-sm font-semibold first:mt-0" {...props} />,
          p: (props) => <p className="my-2.5 first:mt-0 last:mb-0" {...props} />,
          ul: (props) => <ul className="my-2.5 list-disc space-y-1 pl-5 first:mt-0 last:mb-0" {...props} />,
          ol: (props) => <ol className="my-2.5 list-decimal space-y-1 pl-5 first:mt-0 last:mb-0" {...props} />,
          li: (props) => <li className="pl-0.5 [&>p]:my-1" {...props} />,
          a: (props) => (
            <a
              className="font-medium text-primary underline underline-offset-2"
              target="_blank"
              rel="noreferrer noopener"
              {...props}
            />
          ),
          blockquote: (props) => (
            <blockquote className="my-2.5 border-l-2 border-border-strong pl-3 text-muted-foreground" {...props} />
          ),
          hr: () => <hr className="my-4" />,
          pre: (props) => <pre className="code-block my-2.5 [&>code]:bg-transparent [&>code]:p-0" {...props} />,
          code: (props) => (
            <code className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]" {...props} />
          ),
          table: (props) => (
            <div className="my-2.5 overflow-x-auto rounded-md border">
              {/* The last row drops its bottom border (the wrapper already draws one). */}
              <table className="w-full border-collapse text-[13px] [&_tbody_tr:last-child>td]:border-b-0" {...props} />
            </div>
          ),
          th: (props) => (
            <th className="border-b bg-subtle px-3 py-1.5 text-left font-medium text-muted-foreground" {...props} />
          ),
          td: (props) => <td className="border-b px-3 py-1.5 align-top" {...props} />,
          ...components,
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
