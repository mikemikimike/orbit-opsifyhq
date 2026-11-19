import { Server } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Callout } from "@/components/ui/callout";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { Section } from "@/components/ui/section";
import { Skeleton } from "@/components/ui/skeleton";
import { Status } from "@/components/ui/status";
import type { useWorkers } from "@/lib/queries";
import { brandName } from "@/lib/presentation";
import { relativeTime } from "@/lib/utils";
import { errorMessage } from "@orbit/api-client";

import { targetLabel } from "./shared";

/** Workers that have reported a heartbeat. */
export function LiveWorkersSection({ workers }: { workers: ReturnType<typeof useWorkers> }) {
  const workerList = workers.data ?? [];
  return (
    <Section
      title="Live workers"
      count={workers.data ? workerList.length : undefined}
      description="Workers that have reported a heartbeat. A worker is stale when its heartbeat stops."
    >
      {workers.isLoading ? (
        <Card className="overflow-hidden">
          <div className="px-5 py-4">
            <Skeleton className="h-10" />
          </div>
        </Card>
      ) : workers.isError ? (
        <Callout tone="danger" title="Workers could not be loaded">
          {errorMessage(workers.error)}
        </Callout>
      ) : !workerList.length ? (
        <EmptyState
          icon={<Server />}
          title="No workers are connected"
          description="Launch a profile below, or bootstrap one yourself. A worker appears here after its first heartbeat."
        />
      ) : (
        <Card className="overflow-hidden">
          <div
            aria-hidden
            className="hidden grid-cols-[minmax(0,1fr)_7rem_12rem_9rem] gap-x-4 border-b bg-subtle px-5 py-2 text-xs font-medium text-muted-foreground md:grid"
          >
            <span>Worker</span>
            <span>State</span>
            <span>Harnesses</span>
            <span className="text-right">Last heartbeat</span>
          </div>
          <ul>
            {workerList.map((worker) => (
              <li
                key={worker.workerId}
                className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1.5 border-b px-4 py-3.5 last:border-b-0 sm:px-5 md:grid-cols-[minmax(0,1fr)_7rem_12rem_9rem]"
              >
                <div className="min-w-0">
                  <p className="truncate font-mono text-[13px]" title={worker.workerId}>
                    {worker.workerId}
                  </p>
                  <p className="mt-0.5 truncate text-[13px] text-muted-foreground">
                    {targetLabel(worker.target)} target
                    {worker.hostname ? ` on ${worker.hostname}` : ""}
                  </p>
                </div>
                <Status
                  className="justify-self-end md:justify-self-start"
                  meta={
                    worker.live
                      ? { label: "Live", tone: "success", shape: "solid" }
                      : { label: "Stale", tone: "warning", shape: "alert" }
                  }
                />
                <div className="col-span-2 flex flex-wrap gap-1.5 md:col-span-1">
                  {worker.harnesses.map((item) => (
                    <Badge key={item} variant="secondary">
                      {brandName(item)}
                    </Badge>
                  ))}
                </div>
                <p className="tabular col-span-2 text-xs text-muted-foreground md:col-span-1 md:text-right md:text-[13px]">
                  <span className="md:hidden">Last heartbeat </span>
                  {relativeTime(worker.lastHeartbeatAt)}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </Section>
  );
}
