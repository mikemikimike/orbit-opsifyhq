"use client";

import { useQuery } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Callout } from "@/components/ui/callout";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api } from "@/lib/api";
import { copyText } from "@/lib/browser";
import { unwrap } from "@/lib/queries";
import type { WorkerBootstrap, WorkerProfile } from "@/lib/types";
import { errorMessage } from "@orbit/api-client";

const MODES = [
  { value: "docker", label: "Docker" },
  { value: "compose", label: "Compose" },
  { value: "kubernetes", label: "Kubernetes" },
] as const;

/** Docker, Compose and Kubernetes snippets for starting a worker from a profile by hand. */
export function BootstrapDialog({
  profile,
  onClose,
}: {
  profile: WorkerProfile | null;
  onClose: () => void;
}) {
  const bootstrap = useQuery<WorkerBootstrap>({
    queryKey: ["worker-bootstrap", profile?.id],
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/worker-profiles/{profile_id}/bootstrap", {
          params: { path: { profile_id: profile!.id } },
        }),
      ),
    enabled: Boolean(profile),
  });

  async function copy(value: string) {
    if (await copyText(value)) toast.success("Copied");
    else toast.error("Copy unavailable");
  }

  return (
    <Dialog open={Boolean(profile)} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Bootstrap {profile?.name}</DialogTitle>
          <DialogDescription>
            The output sets <code className="font-mono text-[13px]">ORBIT_WORKER_PROFILE_ID</code>{" "}
            for you. Supply the database, NATS, master key and harness auth from your secret store.
          </DialogDescription>
        </DialogHeader>
        {bootstrap.isLoading ? (
          <Skeleton className="h-64" />
        ) : bootstrap.data ? (
          <Tabs defaultValue="docker">
            <TabsList>
              {MODES.map((mode) => (
                <TabsTrigger key={mode.value} value={mode.value}>
                  {mode.label}
                </TabsTrigger>
              ))}
            </TabsList>
            {MODES.map((mode) => (
              <TabsContent key={mode.value} value={mode.value} className="mt-4">
                <div className="relative">
                  <Button
                    size="sm"
                    variant="outline"
                    className="absolute right-2 top-2 z-10"
                    onClick={() => void copy(bootstrap.data![mode.value])}
                  >
                    <Copy /> Copy {mode.label}
                  </Button>
                  <pre
                    className="code-block max-h-[50vh] pr-4 pt-12"
                    tabIndex={0}
                    aria-label={`${mode.label} bootstrap output`}
                  >
                    {bootstrap.data[mode.value]}
                  </pre>
                </div>
              </TabsContent>
            ))}
            {bootstrap.data.notes.length ? (
              <ul className="mt-4 list-inside list-disc space-y-1 text-[13px] text-muted-foreground">
                {bootstrap.data.notes.map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            ) : null}
          </Tabs>
        ) : (
          <Callout tone="danger" title="Bootstrap output could not be generated">
            {errorMessage(bootstrap.error)}
          </Callout>
        )}
      </DialogContent>
    </Dialog>
  );
}
