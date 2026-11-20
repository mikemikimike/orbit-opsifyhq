import type { ReactNode } from "react";

import { Status } from "@/components/ui/status";
import type { WorkerLaunch, WorkerProfile } from "@/lib/types";

/** Only fresh failures raise the alarm. Old ones stay in History, where they are a record. */
const FRESH_MS = 15 * 60 * 1000;

export function freshFailedLaunches(launches: WorkerLaunch[]): WorkerLaunch[] {
  return launches.filter(
    (item) => item.status === "failed" && Date.now() - new Date(item.updatedAt).getTime() < FRESH_MS,
  );
}

/** The at-a-glance facts shown under the page title. */
export function workersSummary({
  workers,
  profiles,
  failedLaunches,
}: {
  workers: Array<{ live: boolean }> | undefined;
  profiles: WorkerProfile[] | undefined;
  failedLaunches: WorkerLaunch[];
}): ReactNode[] {
  const summary: ReactNode[] = [];
  if (workers) {
    const liveCount = workers.filter((worker) => worker.live).length;
    const staleCount = workers.length - liveCount;
    summary.push(
      <Status
        key="live"
        meta={
          liveCount
            ? { label: `${liveCount} live ${liveCount === 1 ? "worker" : "workers"}`, tone: "success", shape: "solid" }
            : { label: "No live workers", tone: "neutral", shape: "dashed" }
        }
      />,
    );
    if (staleCount) {
      summary.push(
        <Status key="stale" meta={{ label: `${staleCount} stale`, tone: "warning", shape: "alert" }} />,
      );
    }
  }
  if (failedLaunches.length) {
    summary.push(
      <Status
        key="failed"
        meta={{
          label: `${failedLaunches.length} failed ${failedLaunches.length === 1 ? "launch" : "launches"}`,
          tone: "danger",
          shape: "cross",
        }}
      />,
    );
  }
  if (profiles) {
    const enabledCount = profiles.filter((profile) => profile.enabled).length;
    summary.push(
      <span key="enabled" className="tabular text-[13px] text-muted-foreground">
        {enabledCount} of {profiles.length} profiles enabled
      </span>,
    );
  }
  return summary;
}
