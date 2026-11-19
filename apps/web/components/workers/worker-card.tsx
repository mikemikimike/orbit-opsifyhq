"use client";

import {
  Boxes,
  Copy,
  Download,
  Ellipsis,
  Pencil,
  Power,
  PowerOff,
  Rocket,
  RotateCw,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import { exportResource } from "@/components/export-resource-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Status } from "@/components/ui/status";
import { copyText } from "@/lib/browser";
import { brandName } from "@/lib/presentation";
import type { WorkerProfile } from "@/lib/types";
import { relativeTime } from "@/lib/utils";

import { targetLabel } from "./shared";

/** One worker profile in the Profiles list. */
export function WorkerCard({
  profile,
  runningCount,
  platformVersion,
  launching,
  launchDisabled,
  relaunching,
  onLaunch,
  onRelaunch,
  onBootstrap,
  onEdit,
  onToggle,
  onDelete,
}: {
  profile: WorkerProfile;
  runningCount: number;
  platformVersion: string | undefined;
  launching: boolean;
  launchDisabled: boolean;
  relaunching: boolean;
  onLaunch: () => void;
  onRelaunch: () => void;
  onBootstrap: () => void;
  onEdit: () => void;
  onToggle: () => void;
  onDelete: () => void;
}) {
  const scoped = (profile as WorkerProfile & { agentScope?: string }).agentScope === "selected";
  const outdated = platformVersion && profile.imageTag && profile.imageTag !== platformVersion;
  return (
    <li className="border-b px-4 py-4 last:border-b-0 sm:px-5">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between lg:gap-6">
        <div className="min-w-0 space-y-1.5">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <p className="min-w-0 break-words text-sm font-semibold">{profile.name}</p>
            <Badge variant={profile.enabled ? "success" : "outline"}>
              {profile.enabled ? "Enabled" : "Disabled"}
            </Badge>
            {runningCount ? (
              <Status
                meta={{
                  label: `${runningCount} running`,
                  tone: "success",
                  shape: "orbiting",
                }}
              />
            ) : null}
          </div>
          <p className="text-[13px] text-muted-foreground">
            {brandName(profile.harnessKind)} worker on {targetLabel(profile.target)}.{" "}
            {profile.executionDriver === "docker"
              ? "Starts a container per execution."
              : "Runs as a long-lived process."}
          </p>
          <p
            className="truncate font-mono text-xs text-muted-foreground"
            title={`${profile.image}:${profile.imageTag || platformVersion || "platform tag"}`}
          >
            {profile.image}:
            {profile.imageTag || `${platformVersion ?? "platform tag"} (follows the platform)`}
          </p>
          <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
            <Badge variant="secondary">{scoped ? `${profile.agentIds.length} agents` : "All agents"}</Badge>
            <Badge variant="secondary">
              {profile.skillSlugs.length} {profile.skillSlugs.length === 1 ? "skill" : "skills"}
            </Badge>
            {profile.authSecretName ? (
              <Badge variant="outline" className="font-mono">
                <span className="truncate">{profile.authSecretName}</span>
              </Badge>
            ) : null}
            {outdated ? (
              <Badge
                variant="warning"
                title={`Profile uses ${profile.imageTag}; platform is ${platformVersion}. Edit and relaunch to update.`}
              >
                Image {profile.imageTag}, platform {platformVersion}
              </Badge>
            ) : null}
            <span className="text-xs text-muted-foreground">Updated {relativeTime(profile.updatedAt)}</span>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2 lg:shrink-0 lg:justify-end">
          {runningCount ? (
            <Button size="sm" variant="outline" loading={relaunching} onClick={onRelaunch}>
              <RotateCw /> Relaunch
            </Button>
          ) : (
            <Button size="sm" loading={launching} disabled={launchDisabled} onClick={onLaunch}>
              <Rocket /> Launch
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={onBootstrap}>
            <Boxes /> Bootstrap
          </Button>
          <Button size="sm" variant="outline" onClick={onEdit}>
            <Pencil /> Edit
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button size="icon" variant="ghost" aria-label={`More actions for ${profile.name}`}>
                <Ellipsis />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                onSelect={async () => {
                  if (await copyText(profile.id)) toast.success("Profile ID copied");
                  else toast.error("Copying is not available in this browser");
                }}
              >
                <Copy /> Copy profile ID
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => void exportResource("worker_profiles", profile.id, profile.name)}>
                <Download /> Export as JSON
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={onToggle}>
                {profile.enabled ? (
                  <>
                    <PowerOff /> Disable profile
                  </>
                ) : (
                  <>
                    <Power /> Enable profile
                  </>
                )}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem destructive disabled={profile.enabled} onSelect={onDelete}>
                <Trash2 />
                {profile.enabled ? "Delete profile (disable first)" : "Delete profile"}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </li>
  );
}
