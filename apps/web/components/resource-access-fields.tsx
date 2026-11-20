"use client";

import { forwardRef, useEffect, useId, useImperativeHandle, useState } from "react";
import { Lock } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Callout } from "@/components/ui/callout";
import { ChoiceCard } from "@/components/ui/choice-card";
import { Field } from "@/components/ui/field";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { listAccessTeams } from "@/lib/access-teams";
import { request } from "@/lib/http";
import { errorMessage } from "@orbit/api-client";
import { useLicense } from "@/lib/queries";

type Sharing = { visibility: string; ownerTeamId?: string | null };

const VISIBILITY_OPTIONS = [
  { value: "org", title: "Organization", description: "Everyone in the organization." },
  { value: "team", title: "Team", description: "Members of the team that owns it." },
  { value: "private", title: "Private", description: "Only you and admins." },
] as const;

export type ResourceAccessHandle = {
  /** Flush pending access edits. Returns false when the access save failed. */
  save: () => Promise<boolean>;
};

type ResourceAccessProps = { resource: string; resourceId?: string };

/**
 * Who can see and use a shareable resource. Lives in the resource's edit form. Access is
 * stored separately from the resource, so the parent form calls the exposed `save()`
 * before saving the resource itself.
 */
export const ResourceAccessFields = forwardRef<ResourceAccessHandle, ResourceAccessProps>(
  function ResourceAccessFields({ resource, resourceId }, ref) {
  const queryClient = useQueryClient();
  const labelId = useId();
  const teamFieldId = useId();
  const license = useLicense();
  const features = license.data?.license.features ?? [];
  // Team access is an enterprise feature: without it the sharing API refuses.
  const enabled = features.includes("teams") && features.includes("resource_permissions");
  const [visibility, setVisibility] = useState("org");
  const [teamId, setTeamId] = useState("");
  const teams = useQuery({
    queryKey: ["access-teams"],
    queryFn: listAccessTeams,
    enabled: enabled && Boolean(resourceId),
  });
  // Strict team isolation (Settings → Access policy) makes the owning team decide who can
  // see a resource, whatever its visibility. Same query key as the settings page.
  const policy = useQuery({
    queryKey: ["access-settings"],
    queryFn: () => request<{ teamIsolation: boolean }>("/api/v1/settings/access"),
    enabled: enabled && Boolean(resourceId),
  });
  const sharing = useQuery({
    queryKey: ["sharing", resource, resourceId],
    queryFn: () => request<Sharing>(`/api/v1/sharing/${resource}/${resourceId}`),
    enabled: enabled && Boolean(resourceId),
  });

  useEffect(() => {
    if (sharing.data) {
      setVisibility(sharing.data.visibility);
      setTeamId(sharing.data.ownerTeamId ?? "");
    }
  }, [sharing.data]);

  const save = useMutation({
    mutationFn: () =>
      request<void>(`/api/v1/sharing/${resource}/${resourceId}`, {
        method: "PATCH",
        body: JSON.stringify({ visibility, ownerTeamId: teamId || null }),
      }),
    onSuccess: () => {
      toast.success("Access updated");
      void queryClient.invalidateQueries({ queryKey: ["sharing", resource, resourceId] });
      void queryClient.invalidateQueries({ queryKey: [resource] });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const changed =
    sharing.data !== undefined &&
    (sharing.data.visibility !== visibility || (sharing.data.ownerTeamId ?? "") !== teamId);

  useImperativeHandle(
    ref,
    () => ({
      async save() {
        if (!enabled || !resourceId || !changed || license.isLoading) return true;
        if (visibility === "team" && !teamId) {
          toast.error("Choose the team that owns this resource");
          return false;
        }
        try {
          await save.mutateAsync();
          return true;
        } catch {
          return false;
        }
      },
    }),
    [changed, enabled, license.isLoading, resourceId, save, teamId, visibility],
  );

  if (license.isLoading) return null;

  if (!enabled) {
    return (
      <div className="flex gap-2.5 rounded-lg border bg-subtle px-3 py-2.5 text-[13px] text-muted-foreground">
        <Lock aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <p>
          <span className="font-medium text-foreground">Access</span> · Visible to the whole
          organization. Team and private access need an enterprise license with teams and resource
          permissions.
        </p>
      </div>
    );
  }

  if (!resourceId) {
    return (
      <p className="text-[13px] text-muted-foreground">
        Access can be set after the first save. Until then it is visible to the whole organization.
      </p>
    );
  }

  const ownerTeam = (teams.data ?? []).find((team) => team.id === teamId);
  const isolated = Boolean(policy.data?.teamIsolation && ownerTeam);

  return (
    <div className="space-y-3 rounded-lg border p-3 sm:p-4">
      <div>
        <p id={labelId} className="text-sm font-medium">
          Access
        </p>
        <p className="mt-0.5 text-[13px] text-muted-foreground">
          Who can see and use this. Organization admins always can.
        </p>
      </div>
      {sharing.isLoading ? (
        <Skeleton className="h-16" />
      ) : (
        <>
          <div role="radiogroup" aria-labelledby={labelId} className="grid gap-2 sm:grid-cols-3">
            {VISIBILITY_OPTIONS.map((option) => (
              <ChoiceCard
                key={option.value}
                selected={visibility === option.value}
                onSelect={() => setVisibility(option.value)}
                title={option.title}
                description={option.description}
              />
            ))}
          </div>
          <Field
            label="Owning team"
            htmlFor={teamFieldId}
            hint={
              visibility === "team"
                ? "Members of this team get access."
                : "Optional. Leads of the owning team can manage it."
            }
          >
            <Select id={teamFieldId} value={teamId} onChange={(event) => setTeamId(event.target.value)}>
              <option value="">{visibility === "team" ? "Select a team" : "No owning team"}</option>
              {(teams.data ?? []).map((team) => (
                <option key={team.id} value={team.id}>
                  {team.name}
                </option>
              ))}
            </Select>
          </Field>
          {isolated && visibility !== "team" ? (
            <Callout tone="neutral">
              Strict team isolation is on, so only members of {ownerTeam?.name} and admins can see
              this. Choose No owning team to share it with everyone, or turn isolation off in
              Settings → Access policy.
            </Callout>
          ) : null}
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => save.mutate()}
              loading={save.isPending}
              disabled={!changed || (visibility === "team" && !teamId)}
            >
              Save access
            </Button>
            <span className="text-[13px] text-muted-foreground">
              {visibility === "team" && !teamId
                ? "Choose the team that owns it."
                : changed
                  ? "Access is saved separately from the rest of this form."
                  : "Saved."}
            </span>
          </div>
        </>
      )}
    </div>
  );
});
