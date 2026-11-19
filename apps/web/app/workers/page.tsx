"use client";

import { useEffect, useState, Suspense, type ReactNode } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Boxes,
  ChevronDown,
  Copy,
  Download,
  Ellipsis,
  Pencil,
  Plus,
  Power,
  PowerOff,
  Rocket,
  RotateCw,
  ScrollText,
  Server,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { ResourceAccessFields } from "@/components/resource-access-fields";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Callout } from "@/components/ui/callout";
import { Card } from "@/components/ui/card";
import { Checkbox, CheckboxField } from "@/components/ui/checkbox";
import { ChoiceCard } from "@/components/ui/choice-card";
import { useConfirm } from "@/components/ui/confirm";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { Section } from "@/components/ui/section";
import { Segmented } from "@/components/ui/segmented";
import { Select } from "@/components/ui/select";
import { exportResource } from "@/components/export-resource-button";
import { ImportPortableButton } from "@/components/import-portable-dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Status, type StatusMeta } from "@/components/ui/status";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { copyText } from "@/lib/browser";
import {
  queryKeys,
  unwrap,
  useAgents,
  useProviders,
  useSkills,
  useWorkerLaunches,
  useConnections,
  useEnvMappings,
  useMeta,
  useWorkerProfileEnvMappings,
  useWorkerProfiles,
  useWorkers,
} from "@/lib/queries";
import {
  launchWorkerProfile,
  stopWorkerLaunch,
  workerLaunchLogs,
} from "@/lib/launcher";
import { errorMessage } from "@orbit/api-client";
import type { WorkerBootstrap, WorkerLaunch, WorkerProfile } from "@/lib/types";
import { cn, relativeTime, titleCase } from "@/lib/utils";
import { brandName } from "@/lib/presentation";
import { DEFAULT_WORKER_IMAGES } from "@/lib/worker-images";

const DEFAULT_IMAGES = DEFAULT_WORKER_IMAGES;
const ACTIVE_STATUSES = ["pending", "starting", "running"];

const TARGET_LABEL: Record<string, string> = {
  docker: "Docker",
  kubernetes: "Kubernetes",
  local: "Local",
};

const LAUNCH_STATUS: Record<string, StatusMeta> = {
  pending: { label: "Pending", tone: "neutral", shape: "dashed" },
  starting: { label: "Starting", tone: "info", shape: "satellite" },
  running: { label: "Running", tone: "success", shape: "orbiting" },
  failed: { label: "Failed", tone: "danger", shape: "cross" },
  stopped: { label: "Stopped", tone: "neutral", shape: "slash" },
};

/** The launcher is optional; when it is missing, say that instead of a transport error. */
function launchErrorText(error: string): string {
  if (/worker launcher request failed|launcher.*error sending request|no worker launcher/i.test(error)) {
    return "ORBIT could not reach a worker launcher, so nothing was started. Start this worker with Bootstrap, or configure the launcher.";
  }
  return error;
}

function launchStatusMeta(status: string): StatusMeta {
  return LAUNCH_STATUS[status] ?? { label: titleCase(status), tone: "neutral", shape: "ring" };
}

function targetLabel(target: string) {
  return TARGET_LABEL[target] ?? titleCase(target);
}

function isActive(launch: { status: string }) {
  return ACTIVE_STATUSES.includes(launch.status);
}

/** A group heading inside a long dialog form. */
function FormGroup({
  title,
  description,
  children,
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <fieldset className="min-w-0 space-y-3 border-t pt-4 first:border-t-0 first:pt-0">
      <legend className="sr-only">{title}</legend>
      <div aria-hidden>
        <h3 className="text-[15px] font-semibold">{title}</h3>
        {description ? <p className="mt-0.5 text-[13px] text-muted-foreground">{description}</p> : null}
      </div>
      {children}
    </fieldset>
  );
}

/** A scrollable list of checkboxes with Select all / Clear helpers. */
function PickList({
  label,
  items,
  selected,
  onChange,
  empty,
  idPrefix,
}: {
  label: string;
  items: Array<{ id: string; label: string; description?: string }>;
  selected: string[];
  onChange: (next: string[]) => void;
  empty: string;
  idPrefix: string;
}) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="tabular text-[13px] text-muted-foreground" aria-live="polite">
          {selected.length} of {items.length} selected
        </p>
        <div className="flex gap-1">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={!items.length}
            aria-label={`Select all ${label}`}
            onClick={() => onChange(items.map((item) => item.id))}
          >
            Select all
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={!selected.length}
            aria-label={`Clear ${label}`}
            onClick={() => onChange([])}
          >
            Clear
          </Button>
        </div>
      </div>
      {items.length ? (
        <ul className="max-h-48 divide-y overflow-y-auto rounded-lg border bg-card">
          {items.map((item) => {
            const id = `${idPrefix}-${item.id}`;
            return (
              <li key={item.id}>
                <label
                  htmlFor={id}
                  className="flex cursor-pointer items-start gap-3 px-3 py-2.5 hover:bg-muted/50"
                >
                  <Checkbox
                    id={id}
                    className="mt-0.5"
                    checked={selected.includes(item.id)}
                    onChange={(event) =>
                      onChange(
                        event.target.checked
                          ? [...selected, item.id]
                          : selected.filter((value) => value !== item.id),
                      )
                    }
                  />
                  <span className="min-w-0">
                    <span className="block truncate text-sm">{item.label}</span>
                    {item.description ? (
                      <span className="block truncate text-[13px] text-muted-foreground">
                        {item.description}
                      </span>
                    ) : null}
                  </span>
                </label>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="rounded-lg border border-dashed p-3 text-[13px] text-muted-foreground">{empty}</p>
      )}
    </div>
  );
}

type InlineMapping = { envVar: string; connectionId: string; secretKey: string };

function WorkersPageInner() {
  const profiles = useWorkerProfiles();
  const [showInactive, setShowInactive] = useState(false);
  const launches = useWorkerLaunches(showInactive);
  const liveWorkers = useWorkers();
  const providers = useProviders();
  const agents = useAgents({ limit: 100 });
  const skills = useSkills();
  const connections = useConnections();
  const meta = useMeta();
  const platformVersion = meta.data?.version;
  const queryClient = useQueryClient();
  const confirm = useConfirm();
  const searchParams = useSearchParams();
  const [open, setOpen] = useState(false);
  const [prefilled, setPrefilled] = useState(false);
  const [editing, setEditing] = useState<WorkerProfile | null>(null);
  const [bootstrapProfile, setBootstrapProfile] = useState<WorkerProfile | null>(null);
  const [logsLaunch, setLogsLaunch] = useState<{ id: string; name: string } | null>(null);
  const [logs, setLogs] = useState("");
  const [logsLoading, setLogsLoading] = useState(false);
  const [logsOpen, setLogsOpen] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const [name, setName] = useState("OpenCode production");
  const [harness, setHarness] = useState("opencode");
  const [target, setTarget] = useState("docker");
  const [driver, setDriver] = useState("process");
  const [image, setImage] = useState(DEFAULT_IMAGES.opencode);
  const [tag, setTag] = useState("");
  const [provider, setProvider] = useState("opencode-go");
  const [model, setModel] = useState("deepseek-v4-pro");
  const [authSecret, setAuthSecret] = useState("");
  const [envMappings, setEnvMappings] = useState<InlineMapping[]>([]);
  const [envMappingIds, setEnvMappingIds] = useState<string[]>([]);
  const [agentScope, setAgentScope] = useState<"all" | "selected">("all");
  const [agentIds, setAgentIds] = useState<string[]>([]);
  const [skillSlugs, setSkillSlugs] = useState<string[]>([]);
  const [cpuRequest, setCpuRequest] = useState("100m");
  const [memoryRequest, setMemoryRequest] = useState("256Mi");
  const [cpuLimit, setCpuLimit] = useState("1");
  const [memoryLimit, setMemoryLimit] = useState("1Gi");
  const [workspaceSize, setWorkspaceSize] = useState("1Gi");
  const [allowWebSearch, setAllowWebSearch] = useState(false);
  const [allowWebFetch, setAllowWebFetch] = useState(false);
  const [allowGlab, setAllowGlab] = useState(false);
  const [allowGh, setAllowGh] = useState(false);
  const [nativeToolsText, setNativeToolsText] = useState("{}");
  const [confirmRelaunch, setConfirmRelaunch] = useState(false);
  /** A reusable mapping to tick when the dialog opens (from Environments → Attach). */
  const [attachMappingId, setAttachMappingId] = useState<string | null>(null);
  const [deepLinked, setDeepLinked] = useState(false);

  // Pre-fill from Work wizard query params.
  if (searchParams.get("create") === "1" && !prefilled) {
    setPrefilled(true);
    const harnessParam = searchParams.get("harness");
    const placementParam = searchParams.get("placement");
    if (harnessParam) setHarness(harnessParam);
    if (placementParam) setTarget(placementParam === "kubernetes" ? "kubernetes" : placementParam === "docker" ? "docker" : "local");
    setOpen(true);
  }

  const providerOptionValue = (candidate: NonNullable<typeof providers.data>[number]) => {
    const config = (candidate.config ?? {}) as Record<string, unknown>;
    return candidate.kind === "harness_managed"
      ? String(config.provider ?? candidate.name)
      : candidate.name;
  };
  const workerProviders = providers.data ?? [];
  const selectedProvider = workerProviders.find(
    (candidate) => providerOptionValue(candidate) === provider,
  );
  const providerModels = (() => {
    const settings = (selectedProvider?.config as Record<string, unknown> | undefined)?.settings as
      | Record<string, unknown>
      | undefined;
    return Array.isArray(settings?.models) ? (settings?.models as string[]) : [];
  })();

  const envMappingLibrary = useEnvMappings();
  const profileEnvMappings = useWorkerProfileEnvMappings(editing?.id);
  useEffect(() => {
    if (editing && profileEnvMappings.data) {
      const attached = profileEnvMappings.data.map((mapping) => mapping.id);
      setEnvMappingIds(
        attachMappingId && !attached.includes(attachMappingId) ? [...attached, attachMappingId] : attached,
      );
    }
  }, [editing, profileEnvMappings.data, attachMappingId]);

  // `?edit=<profile>&attach=<mapping>` opens that profile with the mapping ticked, at the
  // section where it is attached. Used by the Environments page.
  useEffect(() => {
    if (deepLinked || !profiles.data) return;
    const editId = searchParams.get("edit");
    if (!editId) return;
    setDeepLinked(true);
    // Consume the link so a reload or Back does not reopen the dialog.
    window.history.replaceState(null, "", window.location.pathname);
    const profile = profiles.data.find((candidate) => candidate.id === editId);
    if (!profile) {
      toast.error("That worker profile no longer exists");
      return;
    }
    const attach = searchParams.get("attach");
    openEdit(profile);
    if (attach) {
      setAttachMappingId(attach);
      setShowAdvanced(true);
      window.setTimeout(() => {
        document.getElementById("wp-reusable-mappings")?.scrollIntoView({ block: "center" });
      }, 300);
    }
    // openEdit is a plain function recreated every render; running once is the point.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deepLinked, profiles.data, searchParams]);

  const activeProfileLaunches = (launches.data ?? []).filter(
    (item) => editing && item.profileId === editing.id && isActive(item),
  );

  const nativeToolsError = (() => {
    try {
      JSON.parse(nativeToolsText || "{}");
      return null;
    } catch {
      return "This is not valid JSON.";
    }
  })();

  function restartFieldsChanged() {
    if (!editing) return false;
    const existingResources = ((editing as WorkerProfile & { resources?: Record<string, unknown> }).resources ?? {}) as {
      requests?: { cpu?: string; memory?: string };
      limits?: { cpu?: string; memory?: string };
      workspace?: { sizeLimit?: string };
    };
    return (
      editing.image !== image ||
      editing.imageTag !== tag ||
      editing.target !== target ||
      editing.executionDriver !== driver ||
      editing.harnessKind !== harness ||
      (editing.authSecretName ?? "") !== authSecret ||
      (editing.provider ?? "") !== (provider || "") ||
      (editing.model ?? "") !== (model || "") ||
      JSON.stringify(existingResources) !==
        JSON.stringify({
          requests: { cpu: cpuRequest || undefined, memory: memoryRequest || undefined },
          limits: { cpu: cpuLimit || undefined, memory: memoryLimit || undefined },
          workspace: { sizeLimit: workspaceSize || undefined },
        })
    );
  }

  function requestSave() {
    if (editing && activeProfileLaunches.length && restartFieldsChanged()) {
      setConfirmRelaunch(true);
      return;
    }
    save.mutate(false);
  }

  const save = useMutation({
    mutationFn: async (applyNow: boolean) => {
      const body = {
        name,
        harnessKind: harness,
        target,
        executionDriver: driver,
        image,
        imageTag: tag,
        agentScope,
        agentIds: agentScope === "selected" ? agentIds : [],
        skillSlugs,
        provider: provider || undefined,
        model: model || undefined,
        authSecretName: authSecret || undefined,
        env: editing?.env ?? {},
        envMappings: envMappings.map((m) => ({
          envVar: m.envVar,
          connectionId: m.connectionId || null,
          secretKey: m.secretKey || null,
        })),
        envMappingIds,
        allowWebSearch,
        allowWebFetch,
        allowGlab,
        allowGh,
        nativeToolPermissions: JSON.parse(nativeToolsText || "{}"),
        resources: {
          requests: { cpu: cpuRequest || undefined, memory: memoryRequest || undefined },
          limits: { cpu: cpuLimit || undefined, memory: memoryLimit || undefined },
          workspace: { sizeLimit: workspaceSize || undefined },
        },
      };
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
          editing && activeProfileLaunches.length && restartFieldsChanged()
            ? { description: "Relaunch the worker to apply the new configuration." }
            : undefined,
        );
      }
      setOpen(false);
      setEditing(null);
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerProfiles });
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerLaunches });
      void queryClient.invalidateQueries({ queryKey: ["worker-profile-env-mappings"] });
      setAttachMappingId(null);
      if (!editing) setBootstrapProfile(profile as WorkerProfile);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  function openCreate() {
    setEditing(null);
    setAttachMappingId(null);
    setName("OpenCode production");
    setHarness("opencode");
    setTarget("docker");
    setDriver("process");
    setImage(DEFAULT_IMAGES.opencode);
    setTag("");
    setProvider("opencode-go");
    setModel("deepseek-v4-pro");
    setAuthSecret("");
    setEnvMappings([]);
    setEnvMappingIds([]);
    setAgentScope("all");
    setAgentIds([]);
    setSkillSlugs([]);
    setCpuRequest("100m");
    setMemoryRequest("256Mi");
    setCpuLimit("1");
    setMemoryLimit("1Gi");
    setWorkspaceSize("1Gi");
    setAllowWebSearch(false);
    setAllowWebFetch(false);
    setAllowGlab(false);
    setAllowGh(false);
    setNativeToolsText("{}");
    setShowAdvanced(false);
    setOpen(true);
  }

  function openEdit(profile: WorkerProfile) {
    setEditing(profile);
    setName(profile.name);
    setHarness(profile.harnessKind);
    setTarget(profile.target);
    setDriver(profile.executionDriver);
    setImage(profile.image);
    setTag(profile.imageTag);
    setProvider(profile.provider ?? "");
    setModel(profile.model ?? "");
    setAuthSecret(profile.authSecretName ?? "");
    // Older installs stored `{}` here instead of a list; treat anything but a list as empty.
    const rawMappings: unknown = (profile as { envMappings?: unknown }).envMappings;
    const mappings = Array.isArray(rawMappings)
      ? (rawMappings as Array<{ envVar?: string; connectionId?: string | null; secretKey?: string | null }>)
      : [];
    setEnvMappings(
      mappings.map((m) => ({ envVar: m.envVar ?? "", connectionId: m.connectionId ?? "", secretKey: m.secretKey ?? "" })),
    );
    const scoped = (profile as WorkerProfile & { agentScope?: string }).agentScope === "selected";
    setAgentScope(scoped ? "selected" : "all");
    setEnvMappingIds([]);
    setAllowWebSearch(profile.allowWebSearch);
    setAllowWebFetch(profile.allowWebFetch);
    setAllowGlab(profile.allowGlab);
    setAllowGh(profile.allowGh);
    setNativeToolsText(JSON.stringify(profile.nativeToolPermissions ?? {}, null, 2));
    setAgentIds(profile.agentIds);
    setSkillSlugs(profile.skillSlugs);
    const resources = ((profile as WorkerProfile & { resources?: Record<string, unknown> }).resources ?? {}) as {
      requests?: { cpu?: string; memory?: string };
      limits?: { cpu?: string; memory?: string };
      workspace?: { sizeLimit?: string };
    };
    setCpuRequest(resources.requests?.cpu ?? "100m");
    setMemoryRequest(resources.requests?.memory ?? "256Mi");
    setCpuLimit(resources.limits?.cpu ?? "1");
    setMemoryLimit(resources.limits?.memory ?? "1Gi");
    setWorkspaceSize(resources.workspace?.sizeLimit ?? "1Gi");
    setShowAdvanced(false);
    setAttachMappingId(null);
    setOpen(true);
  }

  const launch = useMutation({
    mutationFn: launchWorkerProfile,
    onSuccess: () => {
      toast.success("Launch requested");
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerLaunches });
    },
    onError: (error) => {
      const message = errorMessage(error);
      // The launcher is an optional service; its absence should not read as a crash.
      toast.error(
        /launcher/i.test(message)
          ? "No worker launcher is reachable, so ORBIT cannot start this worker itself. Use Bootstrap to start it by hand, or configure the launcher."
          : `Launch failed: ${message}`,
      );
    },
  });

  const relaunch = useMutation({
    mutationFn: async (profileId: string) => {
      const active = (launches.data ?? []).filter(
        (item) => item.profileId === profileId && isActive(item),
      );
      for (const item of active) {
        await stopWorkerLaunch(item.id);
      }
      return launchWorkerProfile(profileId);
    },
    onSuccess: () => {
      toast.success("Relaunch requested");
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerLaunches });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const stop = useMutation({
    mutationFn: stopWorkerLaunch,
    onSuccess: () => {
      toast.success("Launch stopped");
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerLaunches });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  async function openLogs(launchId: string, launchName: string) {
    setLogsLaunch({ id: launchId, name: launchName });
    setLogsOpen(true);
    setLogs("");
    setLogsLoading(true);
    try {
      const result = await workerLaunchLogs(launchId);
      setLogs(result.logs || "No log output yet.");
    } catch (error) {
      const message = errorMessage(error);
      setLogs(
        /worker launcher request failed|launcher.*error sending request|no worker launcher/i.test(message)
          ? "No logs: ORBIT could not reach the worker launcher, which keeps them. Logs from a worker you started yourself are in its own container or process output."
          : message,
      );
    } finally {
      setLogsLoading(false);
    }
  }

  const toggle = useMutation({
    mutationFn: async ({ id, enabled }: { id: string; enabled: boolean }) =>
      unwrap(
        await api.POST(
          enabled
            ? "/api/v1/worker-profiles/{profile_id}/enable"
            : "/api/v1/worker-profiles/{profile_id}/disable",
          { params: { path: { profile_id: id } } },
        ),
      ),
    onSuccess: (_data, variables) => {
      toast.success(variables.enabled ? "Profile enabled" : "Profile disabled");
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerProfiles });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const remove = useMutation({
    mutationFn: async (id: string) =>
      unwrap(
        await api.DELETE("/api/v1/worker-profiles/{profile_id}", {
          params: { path: { profile_id: id } },
        }),
      ),
    onSuccess: () => {
      toast.success("Profile deleted");
      void queryClient.invalidateQueries({ queryKey: queryKeys.workerProfiles });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  async function confirmRelaunchProfile(profile: WorkerProfile) {
    const confirmed = await confirm({
      title: `Relaunch ${profile.name}?`,
      description:
        "The running worker is stopped and started again. Agents running on it may be interrupted.",
      confirmLabel: "Relaunch worker",
      destructive: true,
    });
    if (confirmed) relaunch.mutate(profile.id);
  }

  async function confirmDeleteProfile(profile: WorkerProfile) {
    const confirmed = await confirm({
      title: `Delete ${profile.name}?`,
      description: "The profile and its settings are removed. This cannot be undone.",
      confirmLabel: "Delete profile",
      destructive: true,
    });
    if (confirmed) remove.mutate(profile.id);
  }

  // Derived state for the at-a-glance summary.
  const workerList = liveWorkers.data ?? [];
  const liveCount = workerList.filter((worker) => worker.live).length;
  const staleCount = workerList.length - liveCount;
  const launchList = launches.data ?? [];
  // Only fresh failures raise the alarm. Old ones stay in History, where they are a
  // record rather than something to act on now.
  const FRESH_MS = 15 * 60 * 1000;
  const failedLaunches = launchList.filter(
    (item) => item.status === "failed" && Date.now() - new Date(item.updatedAt).getTime() < FRESH_MS,
  );
  const enabledCount = (profiles.data ?? []).filter((profile) => profile.enabled).length;

  const summary: ReactNode[] = [];
  if (liveWorkers.data) {
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
        <Status
          key="stale"
          meta={{ label: `${staleCount} stale`, tone: "warning", shape: "alert" }}
        />,
      );
    }
  }
  if (failedLaunches.length) {
    summary.push(
      <Status
        key="failed"
        meta={{ label: `${failedLaunches.length} failed ${failedLaunches.length === 1 ? "launch" : "launches"}`, tone: "danger", shape: "cross" }}
      />,
    );
  }
  if (profiles.data) {
    summary.push(
      <span key="enabled" className="tabular text-[13px] text-muted-foreground">
        {enabledCount} of {profiles.data.length} profiles enabled
      </span>,
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Workers"
        description="Workers run agent executions. A profile fixes the image, target, agents, skills and harness auth; launch it from here or bootstrap it yourself with Docker, Compose or Kubernetes."
        meta={summary.length ? summary : undefined}
        actions={
          <div className="flex flex-wrap gap-2">
            <ImportPortableButton />
          <Button onClick={openCreate}>
            <Plus /> New profile
          </Button>
          </div>
        }
      />

      {failedLaunches.length ? (
        <Callout
          tone="danger"
          title={`${failedLaunches.length} ${failedLaunches.length === 1 ? "launch has" : "launches have"} failed`}
          action={
            showInactive ? undefined : (
              <Button size="sm" variant="outline" onClick={() => setShowInactive(true)}>
                Show history
              </Button>
            )
          }
        >
          {failedLaunches[0].error
            ? `${failedLaunches[0].name}: ${launchErrorText(failedLaunches[0].error)}`
            : `${failedLaunches[0].name} did not start. Open its logs for details.`}
        </Callout>
      ) : null}

      <Section
        title="Live workers"
        count={liveWorkers.data ? workerList.length : undefined}
        description="Workers that have reported a heartbeat. A worker is stale when its heartbeat stops."
      >
        {liveWorkers.isLoading ? (
          <Card className="overflow-hidden">
            <div className="px-5 py-4">
              <Skeleton className="h-10" />
            </div>
          </Card>
        ) : liveWorkers.isError ? (
          <Callout tone="danger" title="Workers could not be loaded">
            {errorMessage(liveWorkers.error)}
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
              <Button onClick={openCreate}>
                <Plus /> New profile
              </Button>
            }
          />
        ) : (
          <Card className="overflow-hidden">
            <ul>
              {profiles.data.map((profile) => {
                const running = launchList.filter(
                  (item) => item.profileId === profile.id && isActive(item),
                );
                const scoped =
                  (profile as WorkerProfile & { agentScope?: string }).agentScope === "selected";
                const outdated =
                  platformVersion && profile.imageTag && profile.imageTag !== platformVersion;
                return (
                  <li key={profile.id} className="border-b px-4 py-4 last:border-b-0 sm:px-5">
                    <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between lg:gap-6">
                      <div className="min-w-0 space-y-1.5">
                        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                          <p className="min-w-0 break-words text-sm font-semibold">{profile.name}</p>
                          <Badge variant={profile.enabled ? "success" : "outline"}>
                            {profile.enabled ? "Enabled" : "Disabled"}
                          </Badge>
                          {running.length ? (
                            <Status
                              meta={{
                                label: `${running.length} running`,
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
                          <Badge variant="secondary">
                            {scoped ? `${profile.agentIds.length} agents` : "All agents"}
                          </Badge>
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
                          <span className="text-xs text-muted-foreground">
                            Updated {relativeTime(profile.updatedAt)}
                          </span>
                        </div>
                      </div>

                      <div className="flex flex-wrap items-center gap-2 lg:shrink-0 lg:justify-end">
                        {running.length ? (
                          <Button
                            size="sm"
                            variant="outline"
                            loading={relaunch.isPending && relaunch.variables === profile.id}
                            onClick={() => void confirmRelaunchProfile(profile)}
                          >
                            <RotateCw /> Relaunch
                          </Button>
                        ) : (
                          <Button
                            size="sm"
                            loading={launch.isPending && launch.variables === profile.id}
                            disabled={launch.isPending}
                            onClick={() => launch.mutate(profile.id)}
                          >
                            <Rocket /> Launch
                          </Button>
                        )}
                        <Button size="sm" variant="outline" onClick={() => setBootstrapProfile(profile)}>
                          <Boxes /> Bootstrap
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => openEdit(profile)}>
                          <Pencil /> Edit
                        </Button>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              size="icon"
                              variant="ghost"
                              aria-label={`More actions for ${profile.name}`}
                            >
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
                              <Download /> Export as YAML
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              onSelect={() =>
                                toggle.mutate({ id: profile.id, enabled: !profile.enabled })
                              }
                            >
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
                            <DropdownMenuItem
                              destructive
                              disabled={profile.enabled}
                              onSelect={() => void confirmDeleteProfile(profile)}
                            >
                              <Trash2 />
                              {profile.enabled ? "Delete profile (disable first)" : "Delete profile"}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    </div>
                  </li>
                );
              })}
            </ul>
          </Card>
        )}
      </Section>

      <Section
        title="Launches"
        count={launches.data ? launchList.length : undefined}
        description="Worker instances started from a profile by the launcher."
        actions={
          <Segmented
            label="Launches to show"
            value={showInactive ? "history" : "active"}
            onChange={(value) => setShowInactive(value === "history")}
            options={[
              { value: "active", label: "Active" },
              { value: "history", label: "History" },
            ]}
          />
        }
      >
        {launches.isLoading ? (
          <Card className="overflow-hidden">
            <div className="px-5 py-4">
              <Skeleton className="h-12" />
            </div>
          </Card>
        ) : launches.isError ? (
          <Callout tone="danger" title="Launches could not be loaded">
            {errorMessage(launches.error)}
          </Callout>
        ) : !launchList.length ? (
          <EmptyState
            icon={<Rocket />}
            title={showInactive ? "No launches yet" : "No active launches"}
            description={
              showInactive
                ? "Launch an enabled profile to start a worker."
                : "Launch an enabled profile to start a worker, or switch to History to see past launches."
            }
            action={
              showInactive ? undefined : (
                <Button variant="outline" onClick={() => setShowInactive(true)}>
                  Show history
                </Button>
              )
            }
          />
        ) : (
          <Card className="overflow-hidden">
            <ul>
              {launchList.map((item) => (
                <LaunchRow
                  key={item.id}
                  item={item}
                  relaunching={launch.isPending && launch.variables === item.profileId}
                  stopping={stop.isPending && stop.variables === item.id}
                  onLogs={() => void openLogs(item.id, item.name)}
                  onStop={() => stop.mutate(item.id)}
                  onRelaunch={() => launch.mutate(item.profileId)}
                />
              ))}
            </ul>
          </Card>
        )}
      </Section>

      <BootstrapDialog profile={bootstrapProfile} onClose={() => setBootstrapProfile(null)} />

      <Dialog open={logsOpen} onOpenChange={setLogsOpen}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>Logs for {logsLaunch?.name}</DialogTitle>
            <DialogDescription>The last 200 lines from the launcher runtime.</DialogDescription>
          </DialogHeader>
          <pre className="code-block max-h-[60vh] whitespace-pre-wrap break-all" aria-busy={logsLoading}>
            {logsLoading ? "Loading…" : logs}
          </pre>
        </DialogContent>
      </Dialog>

      <Dialog open={confirmRelaunch} onOpenChange={setConfirmRelaunch}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Relaunch the worker now?</DialogTitle>
            <DialogDescription>
              This profile has a running worker and your changes need a restart to apply. Agents
              running on it may be interrupted.
            </DialogDescription>
          </DialogHeader>
          <p className="text-[13px] text-muted-foreground">
            {activeProfileLaunches.length}{" "}
            {activeProfileLaunches.length === 1 ? "launch" : "launches"} will be stopped and
            launched again.
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmRelaunch(false)}>
              Cancel
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                setConfirmRelaunch(false);
                save.mutate(false);
              }}
            >
              Save only
            </Button>
            <Button
              onClick={() => {
                setConfirmRelaunch(false);
                save.mutate(true);
              }}
            >
              Save and relaunch
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={open} onOpenChange={setOpen}>
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
              <Field label="Name" htmlFor="wp-name">
                <Input id="wp-name" value={name} onChange={(event) => setName(event.target.value)} />
              </Field>
              <div className="grid gap-3 sm:grid-cols-2">
                <Field label="Harness" htmlFor="wp-harness">
                  <Select
                    id="wp-harness"
                    value={harness}
                    onChange={(event) => {
                      const nextHarness = event.target.value;
                      setHarness(nextHarness);
                      setImage(DEFAULT_IMAGES[nextHarness] ?? "");
                      const currentConfig = (selectedProvider?.config ?? {}) as Record<string, unknown>;
                      if (selectedProvider && String(currentConfig.harness ?? "") !== nextHarness) {
                        setProvider("");
                        setModel("");
                      }
                      // Auto-suggest auth secret for harness-managed providers.
                      if (nextHarness === "opencode" && !authSecret) {
                        setAuthSecret("orbit-opencode-auth");
                      } else if (nextHarness === "pi" && !authSecret) {
                        setAuthSecret("orbit-pi-auth");
                      } else if (nextHarness === "mock") {
                        setAuthSecret("");
                      }
                    }}
                  >
                    <option value="pi">Pi</option>
                    <option value="opencode">OpenCode</option>
                    <option value="mock">Mock</option>
                  </Select>
                </Field>
                <Field label="Target" htmlFor="wp-target" hint="Where the worker runs.">
                  <Select id="wp-target" value={target} onChange={(event) => setTarget(event.target.value)}>
                    <option value="docker">Docker</option>
                    <option value="kubernetes">Kubernetes</option>
                    <option value="local">Local</option>
                  </Select>
                </Field>
              </div>
            </FormGroup>

            <FormGroup
              title="Model"
              description="Providers and model catalogs come from Models."
            >
              <div className="grid gap-3 sm:grid-cols-2">
                <Field
                  label="Provider"
                  htmlFor="wp-provider"
                  hint="Harness-managed providers use the harness auth secret. ORBIT-managed providers get credentials injected at runtime."
                >
                  {workerProviders.length ? (
                    <div className="space-y-2">
                      <Select
                        id="wp-provider"
                        value={selectedProvider ? provider : "__custom"}
                        onChange={(event) => {
                          if (event.target.value === "__custom") {
                            setProvider("");
                            setModel("");
                            return;
                          }
                          const next = workerProviders.find(
                            (candidate) => providerOptionValue(candidate) === event.target.value,
                          );
                          const config = (next?.config ?? {}) as Record<string, unknown>;
                          const settings = config.settings as Record<string, unknown> | undefined;
                          const catalog = Array.isArray(settings?.models)
                            ? (settings.models as unknown[]).filter(
                                (value): value is string => typeof value === "string",
                              )
                            : [];
                          if (next?.kind === "harness_managed") {
                            const providerHarness = String(config.harness ?? "");
                            if (providerHarness) {
                              setHarness(providerHarness);
                              setImage(DEFAULT_IMAGES[providerHarness] ?? image);
                            }
                          }
                          setProvider(event.target.value);
                          setModel(String(config.model ?? "") || catalog[0] || "");
                        }}
                      >
                        <option value="__custom">Custom provider</option>
                        {workerProviders.some((candidate) => candidate.kind === "harness_managed") ? (
                          <optgroup label="Harness-authenticated (worker handles auth)">
                            {workerProviders
                              .filter((candidate) => candidate.kind === "harness_managed")
                              .map((candidate) => {
                                const config = (candidate.config ?? {}) as Record<string, unknown>;
                                const optionName = providerOptionValue(candidate);
                                const providerHarness = String(config.harness ?? "");
                                return (
                                  <option key={candidate.id} value={optionName}>
                                    {candidate.name}
                                    {providerHarness ? ` (${providerHarness})` : ""}
                                  </option>
                                );
                              })}
                          </optgroup>
                        ) : null}
                        {workerProviders.some((candidate) => candidate.kind === "openai_compatible") ? (
                          <optgroup label="ORBIT-managed (API key stored in ORBIT)">
                            {workerProviders
                              .filter((candidate) => candidate.kind === "openai_compatible")
                              .map((candidate) => (
                                <option key={candidate.id} value={providerOptionValue(candidate)}>
                                  {candidate.name}
                                </option>
                              ))}
                          </optgroup>
                        ) : null}
                      </Select>
                      {selectedProvider ? null : (
                        <Input
                          value={provider}
                          placeholder="Custom provider name"
                          aria-label="Custom provider name"
                          onChange={(event) => setProvider(event.target.value)}
                        />
                      )}
                    </div>
                  ) : (
                    <Input
                      id="wp-provider"
                      value={provider}
                      placeholder="opencode-go"
                      onChange={(event) => setProvider(event.target.value)}
                    />
                  )}
                </Field>
                <Field
                  label="Model"
                  htmlFor="wp-model"
                  hint={
                    selectedProvider
                      ? "Defaults to the provider's model. Override if needed."
                      : "Free-form model ID for custom or harness-default providers."
                  }
                >
                  {providerModels.length ? (
                    <Select id="wp-model" value={model} onChange={(event) => setModel(event.target.value)}>
                      <option value="">Provider default</option>
                      {model && !providerModels.includes(model) ? (
                        <option value={model}>{model} (custom)</option>
                      ) : null}
                      {providerModels.map((candidate) => (
                        <option key={candidate} value={candidate}>
                          {candidate}
                        </option>
                      ))}
                    </Select>
                  ) : (
                    <Input id="wp-model" value={model} onChange={(event) => setModel(event.target.value)} />
                  )}
                </Field>
              </div>
            </FormGroup>

            <FormGroup
              title="Agents"
              description="Which agents this worker may claim."
            >
              <div role="radiogroup" aria-label="Agent scope" className="grid gap-2 sm:grid-cols-2">
                <ChoiceCard
                  selected={agentScope === "all"}
                  onSelect={() => setAgentScope("all")}
                  title="All agents"
                  description="Agents created later are included automatically."
                />
                <ChoiceCard
                  selected={agentScope === "selected"}
                  onSelect={() => setAgentScope("selected")}
                  title="Selected agents"
                  description="Only the agents you pick."
                />
              </div>
              {agentScope === "selected" ? (
                <PickList
                  label="agents"
                  idPrefix="wp-agent"
                  selected={agentIds}
                  onChange={setAgentIds}
                  items={(agents.data?.items ?? []).map((agent) => ({ id: agent.id, label: agent.name }))}
                  empty="There are no agents to select yet."
                />
              ) : null}
            </FormGroup>

            <FormGroup
              title="Skills"
              description="Shared skills the worker prepares at startup."
            >
              <PickList
                label="skills"
                idPrefix="wp-skill"
                selected={skillSlugs}
                onChange={setSkillSlugs}
                items={(skills.data ?? []).map((skill) => ({ id: skill.slug, label: skill.name }))}
                empty="There are no skills to select yet."
              />
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
                    <Field
                      label="Execution driver"
                      htmlFor="wp-driver"
                      hint="A long-lived process claims many executions. Docker starts a container for each one."
                    >
                      <Select id="wp-driver" value={driver} onChange={(event) => setDriver(event.target.value)}>
                        <option value="process">Process (long-lived worker)</option>
                        <option value="docker">Docker (container per execution)</option>
                      </Select>
                    </Field>
                    <div className="grid gap-3 sm:grid-cols-2">
                      <Field label="Image" htmlFor="wp-image">
                        <Input
                          id="wp-image"
                          className="font-mono text-[13px]"
                          value={image}
                          onChange={(event) => setImage(event.target.value)}
                        />
                      </Field>
                      <Field
                        label="Tag"
                        htmlFor="wp-tag"
                        hint={
                          <>
                            Leave empty to follow the platform&apos;s version; the worker is then
                            relaunched automatically after an upgrade.
                            {platformVersion && tag && tag !== platformVersion ? (
                              <>
                                {" "}
                                <button
                                  type="button"
                                  className="text-primary underline-offset-2 hover:underline"
                                  onClick={() => setTag("")}
                                >
                                  Follow the platform ({platformVersion})
                                </button>
                              </>
                            ) : null}
                          </>
                        }
                      >
                        <Input
                          id="wp-tag"
                          className="font-mono text-[13px]"
                          value={tag}
                          placeholder={platformVersion ? `${platformVersion} (follows the platform)` : "Follows the platform"}
                          onChange={(event) => setTag(event.target.value)}
                        />
                      </Field>
                    </div>
                    <Field
                      label="Harness auth secret"
                      htmlFor="wp-auth"
                      optional
                      hint="Only for harness-managed providers such as OpenCode Go. Leave blank for ORBIT-managed providers; their credentials are injected at runtime."
                    >
                      <Input
                        id="wp-auth"
                        className="font-mono text-[13px]"
                        value={authSecret}
                        onChange={(event) => setAuthSecret(event.target.value)}
                        placeholder="orbit-opencode-auth"
                      />
                    </Field>
                  </FormGroup>

                  <FormGroup title="Resources" description="Requests and limits for the worker container.">
                    <div className="grid gap-3 sm:grid-cols-2">
                      <Field label="CPU request" htmlFor="wp-cpu-request">
                        <Input id="wp-cpu-request" value={cpuRequest} onChange={(event) => setCpuRequest(event.target.value)} placeholder="100m" />
                      </Field>
                      <Field label="Memory request" htmlFor="wp-mem-request">
                        <Input id="wp-mem-request" value={memoryRequest} onChange={(event) => setMemoryRequest(event.target.value)} placeholder="256Mi" />
                      </Field>
                      <Field label="CPU limit" htmlFor="wp-cpu-limit">
                        <Input id="wp-cpu-limit" value={cpuLimit} onChange={(event) => setCpuLimit(event.target.value)} placeholder="1" />
                      </Field>
                      <Field label="Memory limit" htmlFor="wp-mem-limit">
                        <Input id="wp-mem-limit" value={memoryLimit} onChange={(event) => setMemoryLimit(event.target.value)} placeholder="1Gi" />
                      </Field>
                      <Field label="Workspace size" htmlFor="wp-workspace">
                        <Input id="wp-workspace" value={workspaceSize} onChange={(event) => setWorkspaceSize(event.target.value)} placeholder="1Gi" />
                      </Field>
                    </div>
                  </FormGroup>

                  <FormGroup
                    title="OpenCode web tools"
                    description="Off by default so headless OpenCode runs do not stop on an interactive web search request."
                  >
                    <div className="grid gap-2 sm:grid-cols-2">
                      <CheckboxField
                        title="Allow web search"
                        description="Non-interactive. Needs a search provider."
                        checked={allowWebSearch}
                        onChange={(event) => setAllowWebSearch(event.target.checked)}
                      />
                      <CheckboxField
                        title="Allow web fetch"
                        description="Fetch known URLs."
                        checked={allowWebFetch}
                        onChange={(event) => setAllowWebFetch(event.target.checked)}
                      />
                    </div>
                  </FormGroup>

                  <FormGroup
                    title="Command-line tools"
                    description="The image must contain the tool you allow."
                  >
                    <div className="grid gap-2 sm:grid-cols-2">
                      <CheckboxField
                        title="Allow glab"
                        description="Uses GITLAB_TOKEN from a team GitLab connection."
                        checked={allowGlab}
                        onChange={(event) => setAllowGlab(event.target.checked)}
                      />
                      <CheckboxField
                        title="Allow gh"
                        description="Uses GH_TOKEN from a team GitHub connection."
                        checked={allowGh}
                        onChange={(event) => setAllowGh(event.target.checked)}
                      />
                    </div>
                    {allowGlab || allowGh ? (
                      <Callout tone="warning" title="The token is exposed to shell tools">
                        Anything that runs in the worker workspace can read it.
                      </Callout>
                    ) : null}
                  </FormGroup>

                  <FormGroup title="Native tool permissions">
                    <Field
                      label="Permissions for OpenCode"
                      htmlFor="wp-native-tools"
                      error={nativeToolsError}
                      hint="JSON map of tool name to allow, ask or deny. For example, shell allow and bash deny."
                    >
                      <Textarea
                        id="wp-native-tools"
                        className="min-h-24 font-mono text-xs"
                        value={nativeToolsText}
                        onChange={(event) => setNativeToolsText(event.target.value)}
                        aria-invalid={nativeToolsError ? true : undefined}
                        placeholder={'{\n  "bash": "deny",\n  "edit": "allow",\n  "shell": "allow"\n}'}
                      />
                    </Field>
                  </FormGroup>

                  <FormGroup
                    title="Environment variables"
                    description="Credentials the worker fetches at startup and injects into its process and every script it spawns."
                  >
                    <div className="space-y-2">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <p className="text-sm font-medium">Inline mappings</p>
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          onClick={() =>
                            setEnvMappings((current) => [...current, { envVar: "", connectionId: "", secretKey: "" }])
                          }
                        >
                          <Plus /> Add mapping
                        </Button>
                      </div>
                      {envMappings.length === 0 ? (
                        <p className="rounded-lg border border-dashed p-3 text-[13px] text-muted-foreground">
                          No inline mappings. Add one to inject a connection secret as an environment variable.
                        </p>
                      ) : (
                        <ul className="space-y-2">
                          {envMappings.map((mapping, index) => (
                            <li key={index} className="rounded-lg border bg-card p-3">
                              <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)_auto] sm:items-end">
                                <Field label="Variable" htmlFor={`wp-env-${index}-var`}>
                                  <Input
                                    id={`wp-env-${index}-var`}
                                    value={mapping.envVar}
                                    onChange={(e) =>
                                      setEnvMappings((current) =>
                                        current.map((m, i) => (i === index ? { ...m, envVar: e.target.value } : m)),
                                      )
                                    }
                                    placeholder="ENV_VAR_NAME"
                                    className="font-mono text-[13px]"
                                  />
                                </Field>
                                <Field label="Connection" htmlFor={`wp-env-${index}-conn`}>
                                  <Select
                                    id={`wp-env-${index}-conn`}
                                    value={mapping.connectionId}
                                    onChange={(e) =>
                                      setEnvMappings((current) =>
                                        current.map((m, i) => (i === index ? { ...m, connectionId: e.target.value } : m)),
                                      )
                                    }
                                  >
                                    <option value="">Select a connection</option>
                                    {(connections.data ?? []).map((c) => (
                                      <option key={c.id} value={c.id}>
                                        {c.name} ({brandName(c.connectorSlug)})
                                      </option>
                                    ))}
                                  </Select>
                                </Field>
                                <Field label="Secret key" htmlFor={`wp-env-${index}-key`}>
                                  <Input
                                    id={`wp-env-${index}-key`}
                                    value={mapping.secretKey}
                                    onChange={(e) =>
                                      setEnvMappings((current) =>
                                        current.map((m, i) => (i === index ? { ...m, secretKey: e.target.value } : m)),
                                      )
                                    }
                                    placeholder="password"
                                  />
                                </Field>
                                <Button
                                  type="button"
                                  size="icon"
                                  variant="ghost"
                                  className="justify-self-start sm:justify-self-auto"
                                  aria-label={`Remove mapping ${mapping.envVar || index + 1}`}
                                  onClick={() => setEnvMappings((current) => current.filter((_, i) => i !== index))}
                                >
                                  <X />
                                </Button>
                              </div>
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>

                    <div className="space-y-2">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <p id="wp-reusable-mappings" className="text-sm font-medium">
                          Reusable mappings
                        </p>
                        <Link href="/environment-mappings" className="text-[13px] text-primary hover:underline">
                          Manage mappings
                        </Link>
                      </div>
                      {envMappingLibrary.isLoading ? (
                        <Skeleton className="h-16" />
                      ) : !envMappingLibrary.data?.length ? (
                        <p className="rounded-lg border border-dashed p-3 text-[13px] text-muted-foreground">
                          No reusable mappings yet. Create them under Environments and attach them to
                          any profile.
                        </p>
                      ) : (
                        <PickList
                          label="reusable mappings"
                          idPrefix="wp-envmap"
                          selected={envMappingIds}
                          onChange={setEnvMappingIds}
                          items={envMappingLibrary.data.map((mapping) => ({
                            id: mapping.id,
                            label: mapping.name,
                            description: mapping.description || undefined,
                          }))}
                          empty="No reusable mappings."
                        />
                      )}
                    </div>
                  </FormGroup>
                </div>
              ) : null}
            </div>
          </div>

          <ResourceAccessFields resource="worker_profiles" resourceId={editing?.id} />
          <DialogFooter>
            <Button variant="outline" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={requestSave}
              loading={save.isPending}
              disabled={!name.trim() || Boolean(nativeToolsError)}
            >
              {editing ? "Save changes" : "Create profile"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function LaunchRow({
  item,
  relaunching,
  stopping,
  onLogs,
  onStop,
  onRelaunch,
}: {
  item: WorkerLaunch;
  relaunching: boolean;
  stopping: boolean;
  onLogs: () => void;
  onStop: () => void;
  onRelaunch: () => void;
}) {
  const active = isActive(item);
  const relaunchRequired = (item as WorkerLaunch & { relaunchRequired?: boolean }).relaunchRequired;
  return (
    <li className="border-b px-4 py-3.5 last:border-b-0 sm:px-5">
      <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between md:gap-6">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <Status meta={launchStatusMeta(item.status)} />
            <p className="min-w-0 truncate text-sm font-semibold">{item.name}</p>
            {relaunchRequired ? <Badge variant="warning">Relaunch required</Badge> : null}
          </div>
          <p className="text-[13px] text-muted-foreground">
            {targetLabel(item.target)} target.{" "}
            {item.workerId ? (
              <>
                Worker <span className="break-all font-mono text-xs">{item.workerId}</span>
              </>
            ) : (
              "No worker connected from this launch."
            )}
          </p>
          <p className="truncate font-mono text-xs text-muted-foreground" title={item.image}>
            {item.image}
          </p>
          {item.error ? (
            <p className="break-words text-[13px] text-destructive" title={item.error}>
              {launchErrorText(item.error)}
            </p>
          ) : null}
          <p className="text-xs text-muted-foreground">Updated {relativeTime(item.updatedAt)}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2 md:shrink-0">
          {active ? (
            <Button size="sm" variant="outline" loading={stopping} onClick={onStop}>
              <Square /> Stop
            </Button>
          ) : (
            <Button size="sm" variant="outline" loading={relaunching} onClick={onRelaunch}>
              <Rocket /> Launch again
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={onLogs}>
            <ScrollText /> Logs
          </Button>
        </div>
      </div>
    </li>
  );
}

export default function WorkersPage() {
  return (
    <Suspense fallback={<Skeleton className="h-40 rounded-xl" />}>
      <WorkersPageInner />
    </Suspense>
  );
}

function BootstrapDialog({
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

  const modes = [
    { value: "docker", label: "Docker" },
    { value: "compose", label: "Compose" },
    { value: "kubernetes", label: "Kubernetes" },
  ] as const;

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
              {modes.map((mode) => (
                <TabsTrigger key={mode.value} value={mode.value}>
                  {mode.label}
                </TabsTrigger>
              ))}
            </TabsList>
            {modes.map((mode) => (
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
