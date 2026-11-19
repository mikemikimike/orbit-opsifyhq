"use client";

import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ChevronDown } from "lucide-react";
import { toast } from "sonner";

import { ResourceAccessFields } from "@/components/resource-access-fields";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "@/lib/api";
import { launchWorkerProfile, stopWorkerLaunch } from "@/lib/launcher";
import {
  queryKeys,
  unwrap,
  useAgents,
  useConnections,
  useEnvMappings,
  useMeta,
  useProviders,
  useSkills,
  useWorkerProfileEnvMappings,
} from "@/lib/queries";
import type { WorkerLaunch, WorkerProfile } from "@/lib/types";
import { cn } from "@/lib/utils";
import { errorMessage } from "@orbit/api-client";

import { EnvironmentFields } from "./environment-section";
import { RelaunchOnSaveDialog } from "./launch-dialogs";
import { AuthSecretField, ModelFields } from "./model-section";
import { PermissionGroups } from "./permissions-section";
import { ResourceFields } from "./resources-section";
import { BasicsFields, ImageFields } from "./runtime-section";
import { AgentScopeFields, SkillFields } from "./scope-section";
import { FormGroup, isActive } from "./shared";
import {
  DEFAULT_WORKER_FORM,
  buildProfileBody,
  nativeToolsError,
  restartFieldsChanged,
  useWorkerForm,
  valuesFromProfile,
} from "./worker-form";

/** What to open the editor on. `profile: null` creates a new profile. */
export type EditorTarget = {
  profile: WorkerProfile | null;
  /** Pre-fill for a new profile (from the Work wizard). */
  harness?: string;
  target?: string;
  /** A reusable mapping to tick when editing (from Environments, Attach). */
  attachMappingId?: string;
};

export function WorkerEditor({
  editor,
  launches,
  onClose,
  onCreated,
}: {
  editor: EditorTarget;
  launches: WorkerLaunch[];
  onClose: () => void;
  onCreated: (profile: WorkerProfile) => void;
}) {
  const editing = editor.profile;
  const attachMappingId = editor.attachMappingId ?? null;
  const queryClient = useQueryClient();
  const form = useWorkerForm(() =>
    editing
      ? valuesFromProfile(editing)
      : {
          ...DEFAULT_WORKER_FORM,
          harness: editor.harness ?? DEFAULT_WORKER_FORM.harness,
          target: editor.target ?? DEFAULT_WORKER_FORM.target,
        },
  );
  const { values } = form;
  const [showAdvanced, setShowAdvanced] = useState(Boolean(attachMappingId));
  const [confirmRelaunch, setConfirmRelaunch] = useState(false);

  const providers = useProviders();
  const agents = useAgents({ limit: 100 });
  const skills = useSkills();
  const connections = useConnections();
  const meta = useMeta();
  const platformVersion = meta.data?.version;
  const envMappingLibrary = useEnvMappings();
  const profileEnvMappings = useWorkerProfileEnvMappings(editing?.id);

  useEffect(() => {
    if (editing && profileEnvMappings.data) {
      const attached = profileEnvMappings.data.map((mapping) => mapping.id);
      form.set({
        envMappingIds:
          attachMappingId && !attached.includes(attachMappingId) ? [...attached, attachMappingId] : attached,
      });
    }
    // form.set is stable in behaviour (functional update); the data is the trigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editing, profileEnvMappings.data, attachMappingId]);

  useEffect(() => {
    if (!attachMappingId) return;
    const timer = window.setTimeout(() => {
      document.getElementById("wp-reusable-mappings")?.scrollIntoView({ block: "center" });
    }, 300);
    return () => window.clearTimeout(timer);
  }, [attachMappingId]);

  const activeProfileLaunches = launches.filter(
    (item) => editing && item.profileId === editing.id && isActive(item),
  );
  const formNativeToolsError = nativeToolsError(values.nativeToolsText);

  const save = useMutation({
    mutationFn: async (applyNow: boolean) => {
      const body = buildProfileBody(values, editing);
      if (editing) {
        const profile = unwrap(
          await api.PATCH("/api/v1/worker-profiles/{profile_id}", {
            params: { path: { profile_id: editing.id } },
            body,
          }),
        );
        if (applyNow) {
          for (const item of activeProfileLaunches) {
            await stopWorkerLaunch(item.id);
          }
          await launchWorkerProfile(editing.id);
        }
        return profile;
      }
      return unwrap(await api.POST("/api/v1/worker-profiles", { body }));
    },
    onSuccess: (profile, applyNow) => {
      if (applyNow) {
        toast.success("Profile saved and worker relaunched");
      } else {
        toast.success(
          editing ? "Profile saved" : "Profile created",
          editing && activeProfileLaunches.length && restartFieldsChanged(editing, values)
            ? { description: "Relaunch the worker to apply the new configuration." }
            : undefined,
        );
      }
      onClose();
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerProfiles });
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerLaunches });
      void queryClient.invalidateQueries({ queryKey: ["worker-profile-env-mappings"] });
      if (!editing) onCreated(profile as WorkerProfile);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  function requestSave() {
    if (editing && activeProfileLaunches.length && restartFieldsChanged(editing, values)) {
      setConfirmRelaunch(true);
      return;
    }
    save.mutate(false);
  }

  const providerRows = providers.data ?? [];

  return (
    <>
      <RelaunchOnSaveDialog
        open={confirmRelaunch}
        onOpenChange={setConfirmRelaunch}
        launchCount={activeProfileLaunches.length}
        onSaveOnly={() => {
          setConfirmRelaunch(false);
          save.mutate(false);
        }}
        onSaveAndRelaunch={() => {
          setConfirmRelaunch(false);
          save.mutate(true);
        }}
      />

      <Dialog open onOpenChange={(next) => !next && onClose()}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{editing ? `Edit ${editing.name}` : "New worker profile"}</DialogTitle>
            <DialogDescription>
              The worker loads this profile at startup, claims only its agents and prepares its
              skills. Secrets stay outside ORBIT.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-5">
            <FormGroup title="Basics">
              <BasicsFields form={form} providers={providerRows} />
            </FormGroup>

            <FormGroup title="Model" description="Providers and model catalogs come from Models.">
              <ModelFields form={form} providers={providerRows} />
            </FormGroup>

            <FormGroup title="Agents" description="Which agents this worker may claim.">
              <AgentScopeFields form={form} agents={agents.data?.items ?? []} />
            </FormGroup>

            <FormGroup title="Skills" description="Shared skills the worker prepares at startup.">
              <SkillFields form={form} skills={skills.data ?? []} />
            </FormGroup>

            <div className="border-t pt-4">
              <button
                type="button"
                aria-expanded={showAdvanced}
                aria-controls="wp-advanced"
                onClick={() => setShowAdvanced((current) => !current)}
                className="flex w-full items-center justify-between gap-3 rounded-md py-1 text-left"
              >
                <span>
                  <span className="block text-[15px] font-semibold">Advanced</span>
                  <span className="block text-[13px] text-muted-foreground">
                    Image, resources, auth secret, tool access and environment variables.
                  </span>
                </span>
                <ChevronDown
                  className={cn("h-4 w-4 shrink-0 text-muted-foreground transition-transform", showAdvanced && "rotate-180")}
                />
              </button>

              {showAdvanced ? (
                <div id="wp-advanced" className="mt-4 space-y-5">
                  <FormGroup title="Image and runtime">
                    <ImageFields form={form} platformVersion={platformVersion} />
                    <AuthSecretField form={form} />
                  </FormGroup>

                  <FormGroup title="Resources" description="Requests and limits for the worker container.">
                    <ResourceFields form={form} />
                  </FormGroup>

                  <PermissionGroups form={form} />

                  <FormGroup
                    title="Environment variables"
                    description="Credentials the worker fetches at startup and injects into its process and every script it spawns."
                  >
                    <EnvironmentFields
                      form={form}
                      connections={connections.data ?? []}
                      library={envMappingLibrary.data}
                      libraryLoading={envMappingLibrary.isLoading}
                    />
                  </FormGroup>
                </div>
              ) : null}
            </div>
          </div>

          <ResourceAccessFields resource="worker_profiles" resourceId={editing?.id} />
          <DialogFooter>
            <Button variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button
              onClick={requestSave}
              loading={save.isPending}
              disabled={!values.name.trim() || Boolean(formNativeToolsError)}
            >
              {editing ? "Save changes" : "Create profile"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
