"use client";

import { useEffect, useRef, type ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { cn } from "@/lib/utils";

import {
  SECTION_DESCRIPTIONS,
  SECTION_ORDER,
  SECTION_TITLES,
  type EditorIssue,
  type SectionId,
} from "./editor-sections";

type Summary = Record<SectionId, { value: string; missing?: boolean }>;

function issueCount(issues: EditorIssue[], id: SectionId) {
  return issues.filter((issue) => issue.section === id).length;
}

/** Desktop: every section with its current value, so the nav is also the summary. */
export function SectionNav({
  active,
  onSelect,
  summaries,
  issues,
}: {
  active: SectionId;
  onSelect: (id: SectionId) => void;
  summaries: Summary;
  issues: EditorIssue[];
}) {
  return (
    <nav aria-label="Worker profile sections">
      <ol className="space-y-0.5">
        {SECTION_ORDER.map((id, index) => {
          const current = id === active;
          const problems = issueCount(issues, id);
          const summary = summaries[id];
          return (
            <li key={id}>
              <button
                type="button"
                onClick={() => onSelect(id)}
                aria-current={current ? "step" : undefined}
                className={cn(
                  "flex w-full items-start gap-3 rounded-lg px-2.5 py-2 text-left transition-colors",
                  current ? "bg-accent text-accent-foreground" : "hover:bg-muted/60",
                )}
              >
                <span
                  className={cn(
                    "tabular mt-px flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[11px] font-medium",
                    problems
                      ? "bg-destructive/15 text-destructive"
                      : current
                        ? "bg-primary text-primary-foreground"
                        : "bg-muted text-muted-foreground",
                  )}
                >
                  {problems ? "!" : index + 1}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium">{SECTION_TITLES[id]}</span>
                  <span
                    className={cn(
                      "block truncate text-xs",
                      problems
                        ? "text-destructive"
                        : summary.missing
                          ? "text-warning"
                          : "text-muted-foreground",
                    )}
                    title={problems ? undefined : summary.value}
                  >
                    {problems ? `${problems} to fix` : summary.value}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

/** Phone and tablet: the sections as a scrollable row of steps. */
export function SectionPills({
  active,
  onSelect,
  issues,
}: {
  active: SectionId;
  onSelect: (id: SectionId) => void;
  issues: EditorIssue[];
}) {
  // Keep the current step visible in the row (sideways only, never the page).
  const navRef = useRef<HTMLElement>(null);
  useEffect(() => {
    const nav = navRef.current;
    const pill = nav?.querySelector<HTMLElement>(`[data-section="${active}"]`);
    if (!nav || !pill) return;
    const left = pill.offsetLeft - nav.offsetLeft;
    if (left < nav.scrollLeft || left + pill.offsetWidth > nav.scrollLeft + nav.clientWidth) {
      nav.scrollTo({ left: Math.max(0, left - 16) });
    }
  }, [active]);

  return (
    <nav
      ref={navRef}
      aria-label="Worker profile sections"
      className="-mx-4 mb-4 overflow-x-auto px-4 [scrollbar-width:none] lg:hidden [&::-webkit-scrollbar]:hidden"
    >
      <ol className="flex w-max gap-1.5">
        {SECTION_ORDER.map((id, index) => {
          const current = id === active;
          const problems = issueCount(issues, id);
          return (
            <li key={id}>
              <button
                type="button"
                onClick={() => onSelect(id)}
                aria-current={current ? "step" : undefined}
                data-section={id}
                className={cn(
                  "inline-flex h-9 items-center gap-1.5 whitespace-nowrap rounded-full border px-3 text-[13px] font-medium transition-colors",
                  current
                    ? "border-primary/40 bg-accent text-accent-foreground"
                    : "bg-card text-muted-foreground hover:text-foreground",
                  problems && "border-destructive/40 text-destructive",
                )}
              >
                <span className="tabular text-xs opacity-70">{problems ? "!" : index + 1}</span>
                {SECTION_TITLES[id]}
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

/** The section on screen: a titled card that ends with Back and Next. */
export function SectionCard({
  id,
  active,
  onGo,
  children,
}: {
  id: SectionId;
  active: SectionId;
  onGo: (id: SectionId) => void;
  children: ReactNode;
}) {
  if (active !== id) return null;
  const index = SECTION_ORDER.indexOf(id);
  const previous = SECTION_ORDER[index - 1];
  const next = SECTION_ORDER[index + 1];
  return (
    <Card id={`section-${id}`} className="scroll-mt-20" aria-labelledby={`section-${id}-title`}>
      <CardHeader>
        <h2 id={`section-${id}-title`} className="text-[15px] font-semibold leading-6">
          {SECTION_TITLES[id]}
        </h2>
        <p className="text-[13px] text-muted-foreground">{SECTION_DESCRIPTIONS[id]}</p>
      </CardHeader>
      <CardContent className="space-y-5">
        {children}
        <div className="flex items-center justify-between gap-3 border-t pt-4">
          {previous ? (
            <Button type="button" variant="ghost" onClick={() => onGo(previous)}>
              <ChevronLeft /> {SECTION_TITLES[previous]}
            </Button>
          ) : (
            <span />
          )}
          {next ? (
            <Button type="button" variant="outline" onClick={() => onGo(next)}>
              {SECTION_TITLES[next]} <ChevronRight />
            </Button>
          ) : null}
        </div>
      </CardContent>
    </Card>
  );
}
