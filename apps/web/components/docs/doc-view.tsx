"use client";

import { BookOpen } from "lucide-react";

import { DocNav } from "@/components/docs/doc-nav";
import { DocReader } from "@/components/docs/doc-reader";
import { Callout } from "@/components/ui/callout";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { useDocPage, useDocsIndex } from "@/lib/docs";
import { errorMessage } from "@orbit/api-client";

/** Documentation page: index on the left (a disclosure on phones), reader on the right. */
export function DocView({ slug }: { slug: string }) {
  const index = useDocsIndex();
  const page = useDocPage(slug);

  return (
    <div className="space-y-6">
      <PageHeader
        title="Documentation"
        description="The ORBIT handbook, served by this installation. It works offline."
      />
      <div className="grid gap-4 lg:grid-cols-[15rem_minmax(0,1fr)] lg:gap-8">
        <aside>
          <DocNav
            index={index.data}
            loading={index.isLoading}
            current={slug}
            currentTitle={page.data?.title}
          />
        </aside>
        <article className="min-w-0 max-w-4xl">
          {index.isError ? (
            <Callout tone="danger" title="Documentation could not be loaded">
              {errorMessage(index.error)}
            </Callout>
          ) : page.isLoading ? (
            <div className="space-y-3">
              <Skeleton className="h-8 w-2/3" />
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-5/6" />
              <Skeleton className="h-40 w-full" />
            </div>
          ) : page.isError ? (
            <EmptyState
              icon={<BookOpen />}
              title="Page not found"
              description={`There is no documentation page "${slug}". Pick one from the index.`}
            />
          ) : page.data ? (
            <DocReader slug={slug} markdown={page.data.markdown} />
          ) : null}
        </article>
      </div>
    </div>
  );
}
