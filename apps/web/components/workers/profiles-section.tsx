import { Plus, Server } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Callout } from "@/components/ui/callout";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { Section } from "@/components/ui/section";
import { Skeleton } from "@/components/ui/skeleton";
import type { useWorkerProfiles } from "@/lib/queries";
import type { WorkerLaunch, WorkerProfile } from "@/lib/types";
import { errorMessage } from "@orbit/api-client";

import { isActive } from "./shared";
import { WorkerCard } from "./worker-card";

/** Each profile is one worker definition. Launch starts a worker from it. */
export function ProfilesSection({
  profiles,
  launches,
  platformVersion,
  launchPendingFor,
  launchPending,
  relaunchPendingFor,
  onCreate,
  onLaunch,
  onRelaunch,
  onBootstrap,
  onEdit,
  onToggle,
  onDelete,
}: {
  profiles: ReturnType<typeof useWorkerProfiles>;
  launches: WorkerLaunch[];
  platformVersion: string | undefined;
  launchPendingFor: (profile: WorkerProfile) => boolean;
  launchPending: boolean;
  relaunchPendingFor: (profile: WorkerProfile) => boolean;
  onCreate: () => void;
  onLaunch: (profile: WorkerProfile) => void;
  onRelaunch: (profile: WorkerProfile) => void;
  onBootstrap: (profile: WorkerProfile) => void;
  onEdit: (profile: WorkerProfile) => void;
  onToggle: (profile: WorkerProfile) => void;
  onDelete: (profile: WorkerProfile) => void;
}) {
  return (
    <Section
      title="Profiles"
      count={profiles.data?.length}
      description="Each profile is one worker definition. Launch starts a worker from it."
    >
      {profiles.isLoading ? (
        <Card className="overflow-hidden">
          {Array.from({ length: 2 }).map((_, index) => (
            <div key={index} className="border-b px-5 py-4 last:border-b-0">
              <Skeleton className="h-14" />
            </div>
          ))}
        </Card>
      ) : profiles.isError ? (
        <Callout tone="danger" title="Profiles could not be loaded">
          {errorMessage(profiles.error)}
        </Callout>
      ) : !profiles.data?.length ? (
        <EmptyState
          icon={<Server />}
          title="No worker profiles yet"
          description="A profile pins the harness, image, target, agents, skills and auth secret. Workers start with ORBIT_WORKER_PROFILE_ID."
          action={
            <Button onClick={onCreate}>
              <Plus /> New profile
            </Button>
          }
        />
      ) : (
        <Card className="overflow-hidden">
          <ul>
            {profiles.data.map((profile) => (
              <WorkerCard
                key={profile.id}
                profile={profile}
                runningCount={
                  launches.filter((item) => item.profileId === profile.id && isActive(item)).length
                }
                platformVersion={platformVersion}
                launching={launchPendingFor(profile)}
                launchDisabled={launchPending}
                relaunching={relaunchPendingFor(profile)}
                onLaunch={() => onLaunch(profile)}
                onRelaunch={() => onRelaunch(profile)}
                onBootstrap={() => onBootstrap(profile)}
                onEdit={() => onEdit(profile)}
                onToggle={() => onToggle(profile)}
                onDelete={() => onDelete(profile)}
              />
            ))}
          </ul>
        </Card>
      )}
    </Section>
  );
}
