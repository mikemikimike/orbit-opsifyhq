"use client";

import { Suspense } from "react";

import { Skeleton } from "@/components/ui/skeleton";
import { WorkersView } from "@/components/workers/workers-view";
export default function WorkersPage() {
  return (
    <Suspense fallback={<Skeleton className="h-40 rounded-xl" />}>
      <WorkersView />
    </Suspense>
  );
}
