"use client";

import Link from "next/link";
import { ChevronDown } from "lucide-react";

import { Skeleton } from "@/components/ui/skeleton";
import { docRoute, type DocIndex } from "@/lib/docs";
import { cn } from "@/lib/utils";

function NavList({ index, current }: { index: DocIndex; current: string }) {
  return (
    <nav aria-label="Documentation" className="space-y-5">
      {index.categories.map((category) => (
        <div key={category.id}>
          <p className="px-2 text-[13px] font-semibold text-muted-foreground">{category.title}</p>
          <ul className="mt-1 space-y-0.5">
            {category.pages.map((page) => {
              const active = page.slug === current;
              return (
                <li key={page.slug}>
                  <Link
                    href={docRoute(page.slug)}
                    aria-current={active ? "page" : undefined}
                    className={cn(
                      "block min-h-9 rounded-md px-2 py-1.5 text-sm leading-snug transition-colors",
                      active
                        ? "bg-accent font-medium text-accent-foreground"
                        : "text-foreground/80 hover:bg-muted hover:text-foreground",
                    )}
                  >
                    {page.title}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );
}

/**
 * Page index. On phones it is a disclosure above the content (closed by default and
 * re-closed on navigation via `key`); from `lg` it is a sticky sidebar.
 */
export function DocNav({
  index,
  loading,
  current,
  currentTitle,
}: {
  index?: DocIndex;
  loading: boolean;
  current: string;
  currentTitle?: string;
}) {
  if (loading || !index) {
    return <Skeleton className="h-10 w-full rounded-lg lg:h-96" />;
  }
  return (
    <>
      <details key={current} className="group rounded-lg border bg-card lg:hidden">
        <summary className="flex min-h-11 cursor-pointer list-none items-center justify-between gap-2 px-3 text-sm font-medium [&::-webkit-details-marker]:hidden">
          <span className="min-w-0 truncate">{currentTitle ?? "Browse documentation"}</span>
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180" />
        </summary>
        <div className="max-h-[60dvh] overflow-y-auto border-t p-2">
          <NavList index={index} current={current} />
        </div>
      </details>
      <div className="sticky top-4 hidden max-h-[calc(100dvh-2rem)] overflow-y-auto pr-1 lg:block">
        <NavList index={index} current={current} />
      </div>
    </>
  );
}
