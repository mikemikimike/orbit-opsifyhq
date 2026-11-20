"use client";

export const dynamic = 'force-dynamic';

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import Link from "next/link";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown,
  Download,
  Ellipsis,
  ListTodo,
  Pencil,
  Play,
  Plus,
  Trash2,
  Users,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { exportResource } from "@/components/export-resource-button";
import { ImportPortableButton } from "@/components/import-portable-dialog";
import { Badge } from "@/components/ui/badge";
import { Disclosure } from "@/components/ui/disclosure";
import { ResourceAccessFields, type ResourceAccessHandle } from "@/components/resource-access-fields";
import { Button } from "@/components/ui/button";
import { Callout } from "@/components/ui/callout";
import { Card } from "@/components/ui/card";
import { Checkbox, CheckboxField } from "@/components/ui/checkbox";
import { ChoiceCard } from "@/components/ui/choice-card";
import { useConfirm } from "@/components/ui/confirm";
import { DescriptionList } from "@/components/ui/description-list";
import { TagList } from "@/components/ui/tag-list";
import { cleanDescription } from "@/lib/presentation";
import { SkillPicker } from "@/components/skill-picker";
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
  DropdownMenuSeparator,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { Section } from "@/components/ui/section";
import { Segmented } from "@/components/ui/segmented";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { ExecutionStatusBadge } from "@/components/ui/status-badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import {
  queryKeys,
  useAgents,
  useAgentTeams,
  useConnections,
  useEnvMappings,
  useHarnesses,
  useSkills,
  useTeamRuns,
  useWorkerProfiles,
} from "@/lib/queries";
import { errorMessage } from "@orbit/api-client";
import {
  createAgentTeam,
  deleteAgentTeam,
  getTeamRun,
  startTeamRun,
  updateAgentTeam,
  enqueueTeamItems,
  listTeamQueue,
  runTeamQueueItem,
  retryTeamQueueItem,
  requeueTeamQueueItem,
  cancelTeamQueueItem,
  type TeamQueueItem,
  type TeamStepInput,
} from "@/lib/teams";
import type { AgentTeam, TeamRepository, TeamRunDetail, WorkSource } from "@/lib/types";
import { cn, relativeTime } from "@/lib/utils";

const DONE_STEP = new Set(["succeeded", "completed"]);

/** The position of a step in a sequence. */
function StepNumber({ n, className }: { n: number; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        "tabular flex h-6 w-6 shrink-0 items-center justify-center rounded-full border bg-subtle text-xs font-medium text-muted-foreground",
        className,
      )}
    >
      {n}
    </span>
  );
}

/** A titled group inside the long team form. */
function FormSection({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="space-y-3 border-t pt-5 first:border-t-0 first:pt-0">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold">{title}</h3>
          {description ? <p className="mt-0.5 text-[13px] text-muted-foreground">{description}</p> : null}
        </div>
        {actions ? <div className="flex items-center gap-1">{actions}</div> : null}
      </div>
      {children}
    </section>
  );
}

export default function AgentTeamsPage() {
  const accessRef = useRef<ResourceAccessHandle>(null);
  const teams = useAgentTeams();
  const [runsArchived, setRunsArchived] = useState<"active" | "archived" | "all">("active");
  const runs = useTeamRuns(
    runsArchived === "all" ? undefined : runsArchived === "archived",
  );
  const agents = useAgents({ limit: 100 });
  const skills = useSkills();
  const workerProfiles = useWorkerProfiles();
  const connections = useConnections();
  const envMappings = useEnvMappings();
  const harnesses = useHarnesses();
  const queryClient = useQueryClient();
  const confirm = useConfirm();

  const [tab, setTab] = useState<"teams" | "runs">("teams");
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<AgentTeam | null>(null);
  const [mode, setMode] = useState<"pipeline" | "native">("native");
  const [name, setName] = useState("Coding team");
  const [description, setDescription] = useState(
    "",
  );
  const [instructions, setInstructions] = useState(
    "Pick up the next GitHub issue labelled agent-ready, implement it, test it and request review. Repeat until no ready issues remain.",
  );
  const [teamRepository, setTeamRepository] = useState("");
  const [teamBranch, setTeamBranch] = useState("");
  const [workSource, setWorkSource] = useState<WorkSource | null>(null);
  const [repositories, setRepositories] = useState<TeamRepository[]>([]);
  const [workerProfileId, setWorkerProfileId] = useState("");
  const [autoLaunch, setAutoLaunch] = useState(false);
  const [teamSkills, setTeamSkills] = useState<string[]>([]);
  const [teamTimeout, setTeamTimeout] = useState("");
  const [teamConnectionIds, setTeamConnectionIds] = useState<string[]>([]);
  const [teamEnvMappingIds, setTeamEnvMappingIds] = useState<string[]>([]);
  const [steps, setSteps] = useState<TeamStepInput[]>([
    { agentId: "" },
    { agentId: "" },
  ]);
  const [runTeam, setRunTeam] = useState<AgentTeam | null>(null);
  const [task, setTask] = useState("");
  const [repository, setRepository] = useState("");
  const [autoApprove, setAutoApprove] = useState(false);
  const [runHarness, setRunHarness] = useState("");
  const [detail, setDetail] = useState<TeamRunDetail | null>(null);
  const [queueTeam, setQueueTeam] = useState<AgentTeam | null>(null);
  const [queueItems, setQueueItems] = useState<TeamQueueItem[]>([]);
  const [queueText, setQueueText] = useState("");
  const [queueBusy, setQueueBusy] = useState(false);

  const agentOptions = agents.data?.items ?? [];
  const skillOptions = skills.data ?? [];
  const primaryAgent = agentOptions.find((agent) => agent.id === steps[0]?.agentId);
  const nativeInvalid =
    mode === "native" && Boolean(primaryAgent) && primaryAgent?.harness !== "opencode";
  const hasDuplicateAgent =
    steps.every((step) => step.agentId) && new Set(steps.map((step) => step.agentId)).size !== steps.length;
  const canCreate =
    name.trim() &&
    steps.length > 0 &&
    !nativeInvalid &&
    steps.every((step) => step.agentId) &&
    new Set(steps.map((step) => step.agentId)).size === steps.length;
  const blocker = !name.trim()
    ? "Enter a name."
    : !steps.every((step) => step.agentId)
      ? "Choose an agent for every step."
      : hasDuplicateAgent
        ? "Each agent can appear only once."
        : nativeInvalid
          ? "Native mode needs an OpenCode primary agent."
          : null;

  const agentInfo = useMemo(() => {
    const map = new Map(agentOptions.map((agent) => [agent.id, agent]));
    return (id: string) => {
      const agent = map.get(id);
      return { name: agent?.name ?? id.slice(0, 8), harness: agent?.harness ?? null };
    };
  }, [agentOptions]);

  const teamName = (id: string) => teams.data?.find((team) => team.id === id)?.name;

  async function openQueue(team: AgentTeam) {
    setQueueTeam(team);
    setQueueText("");
    setQueueItems([]);
    setQueueBusy(true);
    try { setQueueItems(await listTeamQueue(team.id)); }
    catch (error) { toast.error(errorMessage(error)); }
    finally { setQueueBusy(false); }
  }

  async function enqueueQueue() {
    if (!queueTeam) return;
    const items = queueText.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => {
      const [externalKey, repository] = line.split("|").map((value) => value.trim());
      return { externalKey, repository: repository || undefined, payload: {} };
    });
    if (!items.length) return;
    setQueueBusy(true);
    try {
      await enqueueTeamItems(queueTeam.id, items);
      setQueueItems(await listTeamQueue(queueTeam.id));
      setQueueText("");
      toast.success("Queue updated");
    } catch (error) { toast.error(errorMessage(error)); }
    finally { setQueueBusy(false); }
  }

  async function runPendingQueue() {
    if (!queueTeam) return;
    setQueueBusy(true);
    try {
      for (const item of queueItems.filter((entry) => entry.status === "pending")) {
        setDetail(await runTeamQueueItem(queueTeam.id, item.id));
      }
      setQueueItems(await listTeamQueue(queueTeam.id));
      toast.success("Pending queue items started");
    } catch (error) { toast.error(errorMessage(error)); }
    finally { setQueueBusy(false); }
  }

  async function runQueueItem(item: TeamQueueItem) {
    if (!queueTeam) return;
    setQueueBusy(true);
    try {
      setDetail(await runTeamQueueItem(queueTeam.id, item.id));
      setQueueItems(await listTeamQueue(queueTeam.id));
      toast.success("Queue item started");
    } catch (error) { toast.error(errorMessage(error)); }
    finally { setQueueBusy(false); }
  }

  async function retryQueueItem(item: TeamQueueItem) {
    if (!queueTeam) return;
    setQueueBusy(true);
    try {
      await retryTeamQueueItem(queueTeam.id, item.id);
      setQueueItems(await listTeamQueue(queueTeam.id));
      toast.success("Queue item queued for retry");
    } catch (error) { toast.error(errorMessage(error)); }
    finally { setQueueBusy(false); }
  }

  async function requeueQueueItem(item: TeamQueueItem) {
    if (!queueTeam) return;
    setQueueBusy(true);
    try {
      await requeueTeamQueueItem(queueTeam.id, item.id);
      setQueueItems(await listTeamQueue(queueTeam.id));
      toast.success("Queue item requeued");
    } catch (error) { toast.error(errorMessage(error)); }
    finally { setQueueBusy(false); }
  }

  async function cancelQueueItem(item: TeamQueueItem) {
    if (!queueTeam) return;
    setQueueBusy(true);
    try {
      await cancelTeamQueueItem(queueTeam.id, item.id);
      setQueueItems(await listTeamQueue(queueTeam.id));
      toast.success("Queue item cancelled");
    } catch (error) { toast.error(errorMessage(error)); }
    finally { setQueueBusy(false); }
  }

  const save = useMutation({
    mutationFn: () => {
      const payload = {
        name: name.trim(),
        description,
        mode,
        instructions,
        repository: teamRepository.trim() || undefined,
        branch: teamBranch.trim() || undefined,
        workSource,
        repositories,
        workerProfileId: workerProfileId || undefined,
        autoLaunch,
        skills: teamSkills,
        connectionIds: teamConnectionIds,
        envMappingIds: teamEnvMappingIds,
        timeoutSeconds: teamTimeout ? Number(teamTimeout) : undefined,
        steps,
      };
      return editing
        ? updateAgentTeam(editing.id, payload)
        : createAgentTeam(payload);
    },
    onSuccess: () => {
      toast.success(editing ? "Agent team updated" : "Agent team created");
      setOpen(false);
      setEditing(null);
      void queryClient.invalidateQueries({ queryKey: queryKeys.agentTeams });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  function openCreate() {
    setEditing(null);
    setMode("native");
    setName("Coding team");
    setDescription("");
    setInstructions(
      "Pick up the next GitHub issue labelled agent-ready, implement it, test it and request review. Repeat until no ready issues remain.",
    );
    setTeamRepository("");
    setTeamBranch("");
    setWorkSource(null);
    setRepositories([]);
    setWorkerProfileId("");
    setAutoLaunch(false);
    setTeamSkills([]);
    setTeamTimeout("");
    setTeamConnectionIds([]);
    setTeamEnvMappingIds([]);
    setSteps([{ agentId: "" }, { agentId: "" }]);
    setOpen(true);
  }

  function openEdit(team: AgentTeam) {
    setEditing(team);
    setMode(team.definition.mode ?? "pipeline");
    setName(team.name);
    setDescription(team.description);
    setInstructions(team.definition.instructions ?? "");
    setTeamRepository(team.definition.repository ?? "");
    setTeamBranch(team.definition.branch ?? "");
    setWorkSource(team.definition.workSource ?? null);
    setRepositories(team.definition.repositories ?? []);
    setWorkerProfileId(team.definition.workerProfileId ?? "");
    setAutoLaunch(team.definition.autoLaunch ?? false);
    setTeamSkills(team.definition.skills ?? []);
    setTeamTimeout(
      String((team.definition as unknown as { timeoutSeconds?: number }).timeoutSeconds ?? ""),
    );
    setTeamConnectionIds(
      (team.definition as unknown as { connectionIds?: string[] }).connectionIds ?? [],
    );
    setTeamEnvMappingIds(
      (team.definition as unknown as { envMappingIds?: string[] }).envMappingIds ?? [],
    );
    setSteps((team.definition.steps ?? []).map((step) => ({ agentId: step.agentId })));
    setOpen(true);
  }

  function openRun(team: AgentTeam) {
    setRunHarness("");
    setRunTeam(team);
    setTask(team.definition.instructions ?? "");
    setRepository(team.definition.repository ?? "");
    setAutoApprove(false);
  }

  async function saveTeam() {
    if ((await accessRef.current?.save()) === false) return;
    save.mutate();
  }

  const remove = useMutation({
    mutationFn: deleteAgentTeam,
    onSuccess: () => {
      toast.success("Agent team deleted");
      void queryClient.invalidateQueries({ queryKey: queryKeys.agentTeams });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  async function deleteTeam(team: AgentTeam) {
    const confirmed = await confirm({
      title: `Delete ${team.name}?`,
      description:
        "The team and its queue are removed. Past runs and their executions are kept. This cannot be undone.",
      confirmLabel: "Delete agent team",
      destructive: true,
    });
    if (confirmed) remove.mutate(team.id);
  }

  const run = useMutation({
    mutationFn: () =>
      startTeamRun(runTeam!.id, task.trim(), repository, autoApprove, runHarness || undefined),
    onSuccess: (result) => {
      toast.success("Run started");
      setRunTeam(null);
      setDetail(result);
      setTab("runs");
      void queryClient.invalidateQueries({ queryKey: queryKeys.teamRuns });
      void queryClient.invalidateQueries({ queryKey: queryKeys.executions() });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  async function inspect(runId: string) {
    try {
      setDetail(await getTeamRun(runId));
    } catch (error) {
      toast.error(errorMessage(error));
    }
  }

  useEffect(() => {
    const runId = new URLSearchParams(window.location.search).get("run");
    if (runId) {
      setTab("runs");
      void inspect(runId);
    }
  }, []);

  function patchWorkSource(patch: Partial<WorkSource>) {
    setWorkSource((current) => (current ? { ...current, ...patch } : current));
  }

  const pendingCount = queueItems.filter((item) => item.status === "pending").length;
  const detailDone = detail?.steps.filter((step) => DONE_STEP.has(step.status)).length ?? 0;
  const detailTask = detail ? String(detail.run.input.task ?? "") : "";

  return (
    <div className="space-y-6">
      <PageHeader
        title="Agent teams"
        description="A primary agent and its helpers, working toward one goal. Define the team here, start runs, and follow each step as it executes."
        actions={
          <div className="flex flex-wrap gap-2">
            <ImportPortableButton />
            <Button onClick={openCreate}>
              <Plus /> New agent team
            </Button>
          </div>
        }
      />

      <Tabs value={tab} onValueChange={(value) => setTab(value as "teams" | "runs")}>
        <TabsList>
          <TabsTrigger value="teams">
            Teams
            {teams.data ? <span className="tabular text-xs text-muted-foreground">{teams.data.length}</span> : null}
          </TabsTrigger>
          <TabsTrigger value="runs">Runs</TabsTrigger>
        </TabsList>

        <TabsContent value="teams">
          {teams.isLoading ? (
            <div className="space-y-3">
              <Skeleton className="h-44 rounded-xl" />
              <Skeleton className="h-44 rounded-xl" />
            </div>
          ) : teams.isError ? (
            <Callout tone="danger" title="Could not load agent teams">
              {errorMessage(teams.error)}
            </Callout>
          ) : !teams.data?.length ? (
            <EmptyState
              icon={<Users />}
              title="No agent teams yet"
              description="A team pairs a primary coding agent with builder, tester and reviewer agents that take over in turn."
              action={
                <Button variant="outline" onClick={openCreate}>
                  <Plus /> New agent team
                </Button>
              }
            />
          ) : (
            <div className="space-y-4">
              {teams.data.map((team) => {
                const teamSteps = team.definition.steps ?? [];
                const teamMode = team.definition.mode ?? "pipeline";
                const source = team.definition.workSource;
                const teamSkillList = team.definition.skills ?? [];
                const model = team.definition.model?.model;
                return (
                  <Card key={team.id}>
                    <div className="flex flex-col gap-3 p-4 sm:flex-row sm:items-start sm:justify-between sm:p-5">
                      <div className="min-w-0 space-y-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <h2 className="min-w-0 truncate text-[15px] font-semibold">{team.name}</h2>
                          <Badge variant="outline">
                            {teamMode === "native" ? "Native subagents" : "Pipeline"}
                          </Badge>
                        </div>
                        {team.description ? (
                          <p className="text-[13px] text-muted-foreground">{team.description}</p>
                        ) : null}
                      </div>
                      <div className="flex shrink-0 flex-wrap items-center gap-2">
                        <Button size="sm" onClick={() => openRun(team)}>
                          <Play /> Run
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => void openQueue(team)}>
                          <ListTodo /> Queue
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => openEdit(team)}>
                          <Pencil /> Edit
                        </Button>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button size="icon-sm" variant="ghost" aria-label={`More actions for ${team.name}`}>
                              <Ellipsis />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onSelect={() => void exportResource("agent_teams", team.id, team.name)}>
                              <Download /> Export as YAML
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem destructive onSelect={() => void deleteTeam(team)}>
                              <Trash2 /> Delete agent team
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    </div>

                    <div className="grid gap-5 border-t p-4 sm:p-5 lg:grid-cols-[minmax(0,1fr)_minmax(0,20rem)]">
                      <div className="min-w-0 space-y-2">
                        <h3 className="text-[13px] font-medium text-muted-foreground">
                          {teamMode === "native" ? "Agents" : "Steps in order"}
                        </h3>
                        {teamSteps.length ? (
                          <ol className="space-y-2">
                            {teamSteps.map((step, index) => {
                              const info = agentInfo(step.agentId);
                              return (
                                <li key={`${step.agentId}-${index}`} className="flex items-center gap-3">
                                  <StepNumber n={index + 1} />
                                  <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
                                    <span className="min-w-0 truncate text-sm font-medium">{info.name}</span>
                                    {info.harness ? (
                                      <span className="font-mono text-xs text-muted-foreground">{info.harness}</span>
                                    ) : null}
                                    {index === 0 ? (
                                      <Badge>Primary</Badge>
                                    ) : teamMode === "native" ? (
                                      <Badge variant="secondary">Subagent</Badge>
                                    ) : null}
                                    {step.model?.model ? (
                                      <Badge variant="outline" className="font-mono">
                                        {step.model.provider ? `${step.model.provider}/` : ""}
                                        {step.model.model}
                                      </Badge>
                                    ) : null}
                                  </div>
                                </li>
                              );
                            })}
                          </ol>
                        ) : (
                          <p className="text-[13px] text-muted-foreground">No agents yet. Edit the team to add some.</p>
                        )}
                      </div>
                      <DescriptionList
                        className="min-w-0"
                        items={[
                          team.definition.repository && {
                            label: "Repository",
                            value: team.definition.repository,
                            mono: true,
                          },
                          team.definition.branch && { label: "Branch", value: team.definition.branch, mono: true },
                          team.definition.repositories?.length
                            ? {
                                label: "Workspace",
                                value: `${team.definition.repositories.length} repositories`,
                              }
                            : null,
                          source?.enabled && {
                            label: "Work source",
                            value: (
                              <span className="inline-flex flex-wrap items-center justify-end gap-1.5">
                                {source.provider === "gitlab" ? "GitLab" : "GitHub"}
                                {source.repository ?? source.project ?? source.groupId
                                  ? `: ${source.repository ?? source.project ?? source.groupId}`
                                  : ""}
                                {team.lastPollError ? <Badge variant="warning">Last poll failed</Badge> : null}
                              </span>
                            ),
                          },
                          team.definition.autoLaunch && { label: "Worker", value: "Starts automatically" },
                          model && {
                            label: "Team model",
                            value: `${team.definition.model?.provider ? `${team.definition.model.provider}/` : ""}${model}`,
                            mono: true,
                          },
                          { label: "Updated", value: relativeTime(team.updatedAt) },
                        ]}
                      />
                    </div>
                    {teamSkillList.length ? (
                      <div className="flex flex-col gap-2 border-t px-4 py-3 sm:flex-row sm:items-baseline sm:gap-4 sm:px-5">
                        <h3 className="shrink-0 text-[13px] text-muted-foreground">
                          Skills <span className="tabular">{teamSkillList.length}</span>
                        </h3>
                        <TagList
                          max={8}
                          items={teamSkillList.map((slug) => {
                            const skill = skillOptions.find((item) => item.slug === slug);
                            return {
                              key: slug,
                              label: skill?.name ?? slug,
                              title: cleanDescription(skill?.description) || slug,
                            };
                          })}
                        />
                      </div>
                    ) : null}
                  </Card>
                );
              })}
            </div>
          )}
        </TabsContent>

        <TabsContent value="runs">
          <Section
            title="Runs"
            description="Each run hands work from one agent to the next. Every step is a normal execution."
            actions={
              <Segmented
                label="Show runs"
                value={runsArchived}
                onChange={setRunsArchived}
                options={[
                  { value: "active", label: "Active" },
                  { value: "archived", label: "Archived" },
                  { value: "all", label: "All" },
                ]}
              />
            }
          >
            {runs.isLoading ? (
              <Card className="overflow-hidden">
                {Array.from({ length: 3 }).map((_, index) => (
                  <div key={index} className="border-b px-5 py-4 last:border-b-0">
                    <Skeleton className="h-9" />
                  </div>
                ))}
              </Card>
            ) : runs.isError ? (
              <Callout tone="danger" title="Could not load runs">
                {errorMessage(runs.error)}
              </Callout>
            ) : !runs.data?.length ? (
              <EmptyState
                icon={<Play />}
                title={runsArchived === "active" ? "No runs yet" : `No ${runsArchived === "all" ? "" : "archived "}runs`}
                description={
                  runsArchived === "active"
                    ? (teams.data ?? []).length
                      ? "Start a run from a team on the Teams tab and it will show up here."
                      : "Create a team first; its runs will show up here."
                    : "Nothing matches this filter."
                }
                action={
                  runsArchived !== "active" ? (
                    <Button variant="outline" onClick={() => setRunsArchived("active")}>
                      Show active runs
                    </Button>
                  ) : undefined
                }
              />
            ) : (
              <Card className="overflow-hidden">
                {runs.data.map((item) => {
                  const selected = detail?.run.id === item.id;
                  return (
                    <div
                      key={item.id}
                      className={cn(
                        "flex flex-col gap-2 border-b px-4 py-3.5 last:border-b-0 sm:px-5 md:flex-row md:items-center md:gap-4",
                        selected && "bg-accent/40",
                      )}
                    >
                      <ExecutionStatusBadge status={item.status} className="md:w-32" />
                      <div className="min-w-0 flex-1">
                        <button
                          type="button"
                          onClick={() => void inspect(item.id)}
                          className="block max-w-full truncate rounded text-left text-sm font-medium hover:text-primary hover:underline"
                        >
                          {String(item.input.task ?? "") || "Agent team run"}
                        </button>
                        <p className="truncate text-[13px] text-muted-foreground">
                          {teamName(item.teamId) ?? "Agent team"}, started {relativeTime(item.createdAt)}
                        </p>
                        {item.error ? <p className="break-words text-[13px] text-destructive">{item.error}</p> : null}
                      </div>
                      <Button size="sm" variant="outline" className="self-start md:self-auto" onClick={() => void inspect(item.id)}>
                        View steps
                      </Button>
                    </div>
                  );
                })}
              </Card>
            )}
          </Section>
        </TabsContent>
      </Tabs>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{editing ? `Edit ${editing.name}` : "New agent team"}</DialogTitle>
            <DialogDescription>
              Choose the agents, how they run, and the goal they share. Advanced settings are optional.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-5">
            <FormSection title="Basics">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Name" htmlFor="team-name">
                  <Input id="team-name" value={name} onChange={(event) => setName(event.target.value)} />
                </Field>
                <Field label="Description" htmlFor="team-description">
                  <Input
                    id="team-description"
                    value={description}
                    onChange={(event) => setDescription(event.target.value)}
                  />
                </Field>
              </div>
              <Field
                label="Goal"
                htmlFor="team-goal"
                hint="Given to the primary agent and repeated to every later step along with the previous output."
              >
                <Textarea
                  id="team-goal"
                  className="min-h-20"
                  value={instructions}
                  onChange={(event) => setInstructions(event.target.value)}
                  placeholder="Pick up issues labelled agent-ready, fix them in a loop, and stop when none remain."
                />
              </Field>
            </FormSection>

            <FormSection title="How it runs">
              <div role="radiogroup" aria-label="Execution mode" className="grid gap-2 sm:grid-cols-2">
                <ChoiceCard
                  selected={mode === "native"}
                  onSelect={() => setMode("native")}
                  title="Native subagents"
                  description="One OpenCode execution. The primary agent runs once and the others are its subagents."
                />
                <ChoiceCard
                  selected={mode === "pipeline"}
                  onSelect={() => setMode("pipeline")}
                  title="Pipeline"
                  description="One execution per agent. Each agent takes over in turn and receives the previous output."
                />
              </div>
              {nativeInvalid ? (
                <Callout tone="warning" title="Native mode needs an OpenCode primary agent">
                  Pi and other harnesses can use pipeline mode, or pick an OpenCode agent as the first step.
                </Callout>
              ) : null}
            </FormSection>

            <FormSection
              title="Agents in order"
              description="The first agent is the primary agent. Its own instructions define its role."
              actions={
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setSteps((current) => [...current, { agentId: "" }])}
                >
                  <Plus /> Add agent
                </Button>
              }
            >
              <ol className="space-y-2">
                {steps.map((step, index) => (
                  <li key={index} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-x-2 gap-y-1">
                    <StepNumber n={index + 1} className="mt-1.5" />
                    <div className="min-w-0 space-y-1">
                      <Select
                        aria-label={`Agent for step ${index + 1}`}
                        value={step.agentId}
                        onChange={(event) =>
                          setSteps((current) =>
                            current.map((item, i) =>
                              i === index ? { ...item, agentId: event.target.value } : item,
                            ),
                          )
                        }
                      >
                        <option value="">Select agent…</option>
                        {agentOptions.map((agent) => (
                          <option key={agent.id} value={agent.id}>
                            {agent.name}
                            {agent.harness ? ` (${agent.harness})` : ""}
                          </option>
                        ))}
                      </Select>
                      <p className="text-[13px] text-muted-foreground">
                        {index === 0
                          ? "Primary agent"
                          : mode === "native"
                            ? "Subagent of the primary agent"
                            : `Runs after step ${index}`}
                      </p>
                    </div>
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      aria-label={`Remove step ${index + 1}`}
                      disabled={steps.length <= 1}
                      onClick={() => setSteps((current) => current.filter((_, i) => i !== index))}
                    >
                      <X />
                    </Button>
                  </li>
                ))}
              </ol>
              {hasDuplicateAgent ? (
                <p className="text-[13px] text-destructive" role="alert">
                  Each agent can appear only once in a team.
                </p>
              ) : null}
            </FormSection>

            <FormSection
              title="Repository"
              description="The checkout target for runs. A run can override it; naming a repository only in the goal does not affect checkout."
            >
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Repository" htmlFor="team-repo" optional>
                  <Input
                    id="team-repo"
                    className="font-mono"
                    value={teamRepository}
                    onChange={(event) => setTeamRepository(event.target.value)}
                    placeholder="owner/repo"
                  />
                </Field>
                <Field label="Base branch" htmlFor="team-branch" optional>
                  <Input
                    id="team-branch"
                    className="font-mono"
                    value={teamBranch}
                    onChange={(event) => setTeamBranch(event.target.value)}
                    placeholder="main"
                  />
                </Field>
              </div>
            </FormSection>

            <FormSection
              title="Skills"
              description="Shared by every agent in the team."
            >
              <SkillPicker
                skills={skillOptions}
                value={teamSkills}
                onChange={setTeamSkills}
                empty={
                  <>
                    No skills yet.{" "}
                    <Link href="/skills" className="text-primary hover:underline">
                      Create one
                    </Link>{" "}
                    to share it here.
                  </>
                }
              />
            </FormSection>

            <FormSection title="Advanced" description="Optional. Each group stays closed until you need it.">
              <div className="space-y-2">
                <Disclosure
                  title="Work source"
                  summary={
                    workSource?.enabled
                      ? `Picks up ${workSource.provider === "gitlab" ? "GitLab" : "GitHub"} issues automatically`
                      : "Off"
                  }
                  defaultOpen={Boolean(workSource?.enabled)}
                >
                  <CheckboxField
                    title="Pick up issues automatically"
                    description="Polls the connection, claims matching issues and adds them to the queue."
                    checked={Boolean(workSource?.enabled)}
                    onChange={(event) =>
                      setWorkSource((current) => ({
                        provider: current?.provider ?? "github",
                        enabled: event.target.checked,
                        connectionId: current?.connectionId ?? null,
                        labels: current?.labels ?? ["agent-ready"],
                        claimLabel: current?.claimLabel ?? "agent-in-progress",
                        doneLabel: current?.doneLabel ?? "agent-done",
                        failedLabel: current?.failedLabel ?? "agent-failed",
                        maxItems: current?.maxItems ?? 5,
                        maxAttempts: current?.maxAttempts ?? 1,
                        pollSeconds: current?.pollSeconds ?? 60,
                        autoRun: current?.autoRun ?? false,
                      }))
                    }
                  />
                  {workSource?.enabled ? (
                    <div className="grid gap-4 sm:grid-cols-2">
                      <Field label="Provider" htmlFor="ws-provider">
                        <Select
                          id="ws-provider"
                          value={workSource.provider}
                          onChange={(event) =>
                            patchWorkSource({ provider: event.target.value as WorkSource["provider"] })
                          }
                        >
                          <option value="github">GitHub</option>
                          <option value="gitlab">GitLab</option>
                        </Select>
                      </Field>
                      <Field label="Connection" htmlFor="ws-connection">
                        <Select
                          id="ws-connection"
                          value={workSource.connectionId ?? ""}
                          onChange={(event) => patchWorkSource({ connectionId: event.target.value || null })}
                        >
                          <option value="">Select connection…</option>
                          {(connections.data ?? [])
                            .filter((connection) => connection.connectorSlug === workSource.provider)
                            .map((connection) => (
                              <option key={connection.id} value={connection.id}>
                                {connection.name}
                              </option>
                            ))}
                        </Select>
                      </Field>
                      {workSource.provider === "github" ? (
                        <Field label="Repository" htmlFor="ws-repo">
                          <Input
                            id="ws-repo"
                            className="font-mono"
                            value={workSource.repository ?? ""}
                            onChange={(event) => patchWorkSource({ repository: event.target.value })}
                            placeholder="owner/repo"
                          />
                        </Field>
                      ) : (
                        <>
                          <Field label="Project" htmlFor="ws-project">
                            <Input
                              id="ws-project"
                              className="font-mono"
                              value={workSource.project ?? ""}
                              onChange={(event) => patchWorkSource({ project: event.target.value })}
                              placeholder="group/project"
                            />
                          </Field>
                          <Field label="Group" htmlFor="ws-group" optional hint="Use instead of a project.">
                            <Input
                              id="ws-group"
                              className="font-mono"
                              value={workSource.groupId ?? ""}
                              onChange={(event) => patchWorkSource({ groupId: event.target.value })}
                              placeholder="group"
                            />
                          </Field>
                          <Field label="Board ID" htmlFor="ws-board" optional>
                            <Input
                              id="ws-board"
                              className="font-mono"
                              value={workSource.boardId ?? ""}
                              onChange={(event) =>
                                patchWorkSource({
                                  boardId: event.target.value ? Number(event.target.value) : null,
                                })
                              }
                              placeholder="123"
                            />
                          </Field>
                        </>
                      )}
                      <Field
                        label="Ready labels"
                        htmlFor="ws-labels"
                        hint="Comma-separated. Issues with any of these are picked up."
                        className="sm:col-span-2"
                      >
                        <Input
                          id="ws-labels"
                          value={(workSource.labels ?? []).join(",")}
                          onChange={(event) =>
                            patchWorkSource({
                              labels: event.target.value.split(",").map((label) => label.trim()).filter(Boolean),
                            })
                          }
                          placeholder="agent-ready"
                        />
                      </Field>
                      <div className="grid gap-4 sm:col-span-2 sm:grid-cols-3">
                        <Field label="Max items" htmlFor="ws-max-items">
                          <Input
                            id="ws-max-items"
                            type="number"
                            value={workSource.maxItems ?? 5}
                            onChange={(event) => patchWorkSource({ maxItems: Number(event.target.value) })}
                          />
                        </Field>
                        <Field label="Max attempts" htmlFor="ws-max-attempts">
                          <Input
                            id="ws-max-attempts"
                            type="number"
                            value={workSource.maxAttempts ?? 1}
                            onChange={(event) => patchWorkSource({ maxAttempts: Number(event.target.value) })}
                          />
                        </Field>
                        <Field label="Poll interval (seconds)" htmlFor="ws-poll">
                          <Input
                            id="ws-poll"
                            type="number"
                            value={workSource.pollSeconds ?? 60}
                            onChange={(event) => patchWorkSource({ pollSeconds: Number(event.target.value) })}
                          />
                        </Field>
                      </div>
                      <CheckboxField
                        className="sm:col-span-2"
                        title="Start runs automatically"
                        description="Run pending queue items as soon as they are found."
                        checked={Boolean(workSource.autoRun)}
                        onChange={(event) => patchWorkSource({ autoRun: event.target.checked })}
                      />
                    </div>
                  ) : null}
                </Disclosure>

                <Disclosure
                  title="Multiple repositories"
                  summary={
                    repositories.length
                      ? `${repositories.length} ${repositories.length === 1 ? "repository" : "repositories"} in the workspace`
                      : "Uses the single repository above"
                  }
                  defaultOpen={repositories.length > 0}
                >
                  <p className="text-[13px] text-muted-foreground">
                    Leave empty to use the single repository above. A single primary repository keeps the
                    current root checkout layout.
                  </p>
                  {repositories.map((item, index) => (
                    <div
                      key={index}
                      className="grid gap-2 rounded-lg border p-2.5 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto_auto] sm:items-center"
                    >
                      <Input
                        aria-label={`Repository ${index + 1}`}
                        className="font-mono"
                        value={item.repository}
                        onChange={(event) =>
                          setRepositories((current) =>
                            current.map((entry, i) => (i === index ? { ...entry, repository: event.target.value } : entry)),
                          )
                        }
                        placeholder="owner/repo"
                      />
                      <Input
                        aria-label={`Workspace path for repository ${index + 1}`}
                        className="font-mono"
                        value={item.path ?? ""}
                        onChange={(event) =>
                          setRepositories((current) =>
                            current.map((entry, i) => (i === index ? { ...entry, path: event.target.value } : entry)),
                          )
                        }
                        placeholder="Workspace path, such as lib"
                      />
                      <label className="flex items-center gap-2 text-sm">
                        <Checkbox
                          checked={Boolean(item.primary)}
                          onChange={(event) =>
                            setRepositories((current) =>
                              current.map((entry, i) => (i === index ? { ...entry, primary: event.target.checked } : entry)),
                            )
                          }
                        />
                        Primary
                      </label>
                      <Button
                        type="button"
                        size="icon-sm"
                        variant="ghost"
                        aria-label={`Remove repository ${index + 1}`}
                        onClick={() => setRepositories((current) => current.filter((_, i) => i !== index))}
                      >
                        <X />
                      </Button>
                    </div>
                  ))}
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      setRepositories((current) => [
                        ...current,
                        { repository: "", path: "", primary: current.length === 0, connectionId: null },
                      ])
                    }
                  >
                    <Plus /> Add repository
                  </Button>
                </Disclosure>

                <Disclosure
                  title="Worker and limits"
                  summary={
                    workerProfileId || autoLaunch || teamTimeout
                      ? [
                          autoLaunch ? "Starts a worker automatically" : null,
                          teamTimeout ? `Timeout ${teamTimeout} seconds` : null,
                        ]
                          .filter(Boolean)
                          .join(". ") || "Worker profile chosen"
                      : "Defaults"
                  }
                  defaultOpen={Boolean(workerProfileId || autoLaunch || teamTimeout)}
                >
                  <div className="grid gap-4 sm:grid-cols-2">
                    <Field label="Worker profile" htmlFor="team-worker" optional>
                      <Select
                        id="team-worker"
                        value={workerProfileId}
                        onChange={(event) => setWorkerProfileId(event.target.value)}
                      >
                        <option value="">No automatic worker launch</option>
                        {(workerProfiles.data ?? []).filter((profile) => profile.enabled).map((profile) => (
                          <option key={profile.id} value={profile.id}>
                            {profile.name} ({profile.harnessKind}, {profile.target})
                          </option>
                        ))}
                      </Select>
                    </Field>
                    <Field label="Timeout (seconds)" htmlFor="team-timeout" optional>
                      <Input
                        id="team-timeout"
                        type="number"
                        min={0}
                        value={teamTimeout}
                        onChange={(event) => setTeamTimeout(event.target.value)}
                      />
                    </Field>
                  </div>
                  <CheckboxField
                    title="Start the worker automatically"
                    description="Launches the chosen profile before the first step. Its allowlist must include the team's agents."
                    checked={autoLaunch}
                    onChange={(event) => setAutoLaunch(event.target.checked)}
                  />
                </Disclosure>

                <Disclosure
                  title="Connections and environment"
                  summary={
                    teamConnectionIds.length || teamEnvMappingIds.length
                      ? `${teamConnectionIds.length} connections, ${teamEnvMappingIds.length} environment mappings`
                      : "None selected"
                  }
                  defaultOpen={teamConnectionIds.length + teamEnvMappingIds.length > 0}
                >
                  <div className="space-y-2">
                    <p className="text-[13px] font-medium">Connections</p>
                    {(connections.data ?? []).length ? (
                      <div className="grid max-h-44 gap-2 overflow-y-auto sm:grid-cols-2">
                        {(connections.data ?? []).map((connection) => (
                          <CheckboxField
                            key={connection.id}
                            title={connection.name}
                            description={connection.connectorSlug}
                            checked={teamConnectionIds.includes(connection.id)}
                            onChange={(event) =>
                              setTeamConnectionIds((current) =>
                                event.target.checked
                                  ? [...current, connection.id]
                                  : current.filter((id) => id !== connection.id),
                              )
                            }
                          />
                        ))}
                      </div>
                    ) : (
                      <p className="text-[13px] text-muted-foreground">No connections yet.</p>
                    )}
                  </div>
                  <div className="space-y-2">
                    <p className="text-[13px] font-medium">Environment mappings</p>
                    {(envMappings.data ?? []).length ? (
                      <div className="grid max-h-44 gap-2 overflow-y-auto sm:grid-cols-2">
                        {(envMappings.data ?? []).map((mapping) => (
                          <CheckboxField
                            key={mapping.id}
                            title={mapping.name}
                            checked={teamEnvMappingIds.includes(mapping.id)}
                            onChange={(event) =>
                              setTeamEnvMappingIds((current) =>
                                event.target.checked
                                  ? [...current, mapping.id]
                                  : current.filter((id) => id !== mapping.id),
                              )
                            }
                          />
                        ))}
                      </div>
                    ) : (
                      <p className="text-[13px] text-muted-foreground">No environment mappings yet.</p>
                    )}
                  </div>
                </Disclosure>
              </div>
            </FormSection>
          </div>
          <ResourceAccessFields ref={accessRef} resource="agent_teams" resourceId={editing?.id} />
          <DialogFooter className="sm:items-center">
            {blocker ? <p className="text-[13px] text-muted-foreground sm:mr-auto">{blocker}</p> : null}
            <Button variant="outline" onClick={() => setOpen(false)}>Cancel</Button>
            <Button onClick={() => void saveTeam()} loading={save.isPending} disabled={!canCreate}>
              {editing ? "Save changes" : "Create agent team"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(runTeam)} onOpenChange={(next) => !next && setRunTeam(null)}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Run {runTeam?.name}</DialogTitle>
            <DialogDescription>
              The primary agent receives the task. Later agents receive the previous output.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <Field label="Task for this run" htmlFor="run-task">
              <Textarea
                id="run-task"
                className="min-h-24"
                value={task}
                onChange={(event) => setTask(event.target.value)}
                placeholder={runTeam?.definition.instructions || "Team goal"}
              />
            </Field>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field
                label="Repository"
                htmlFor="run-repo"
                optional
                hint="Leave blank to use the team's repository or the connection's default."
              >
                <Input
                  id="run-repo"
                  className="font-mono"
                  value={repository}
                  onChange={(event) => setRepository(event.target.value)}
                  placeholder="owner/repo"
                />
              </Field>
              <Field label="Harness" htmlFor="run-harness">
                <Select
                  id="run-harness"
                  value={runHarness}
                  onChange={(event) => setRunHarness(event.target.value)}
                >
                  <option value="">Team or agent default</option>
                  {(harnesses.data ?? [])
                    .filter((item) => item.enabled && item.implementationStatus === "available")
                    .map((item) => (
                      <option key={item.kind} value={item.kind}>
                        {item.name}
                      </option>
                    ))}
                </Select>
              </Field>
            </div>
            <div className="space-y-2">
              <Callout tone="warning" title="Auto-approve skips human review">
                Bound GitHub writes run without approval. Capability allowlists still apply. Use it only for
                trusted repositories and tested agents.
              </Callout>
              <CheckboxField
                title="Auto-approve writes for this run"
                description="Level 4 autonomy, for this run only."
                checked={autoApprove}
                onChange={(event) => setAutoApprove(event.target.checked)}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRunTeam(null)}>Cancel</Button>
            <Button
              onClick={() => run.mutate()}
              loading={run.isPending}
              disabled={!task.trim() && !runTeam?.definition.instructions}
            >
              <Play /> Start run
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(queueTeam)} onOpenChange={(next) => !next && setQueueTeam(null)}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>Queue for {queueTeam?.name}</DialogTitle>
            <DialogDescription>
              Work waiting for this team. Items with the same key are added once.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-5">
            <div className="space-y-2">
              <Field
                label="Add items"
                htmlFor="queue-items"
                hint="One per line, as a key or as key|repository."
              >
                <Textarea
                  id="queue-items"
                  className="min-h-24 font-mono text-xs"
                  spellCheck={false}
                  value={queueText}
                  onChange={(event) => setQueueText(event.target.value)}
                  placeholder={"opsifyhq/orbit#6|opsifyhq/orbit\nopsifyhq/orbit#7|opsifyhq/orbit"}
                />
              </Field>
              <Button
                size="sm"
                variant="outline"
                onClick={() => void enqueueQueue()}
                loading={queueBusy}
                disabled={!queueText.trim()}
              >
                <Plus /> Add to queue
              </Button>
            </div>

            <Section
              title="In the queue"
              count={queueItems.length}
              actions={
                <Button
                  size="sm"
                  onClick={() => void runPendingQueue()}
                  loading={queueBusy}
                  disabled={pendingCount === 0}
                >
                  <Play /> Run {pendingCount} pending
                </Button>
              }
            >
              {queueItems.length === 0 ? (
                queueBusy ? (
                  <Skeleton className="h-14" />
                ) : (
                  <EmptyState title="The queue is empty" description="Add items above, or turn on a work source to fill it automatically." />
                )
              ) : (
                <Card className="max-h-80 divide-y overflow-y-auto">
                  {queueItems.map((item) => (
                    <div key={item.id} className="flex flex-col gap-2 px-3 py-3 sm:flex-row sm:items-center sm:gap-3">
                      <div className="min-w-0 flex-1 space-y-0.5">
                        <p className="break-all font-mono text-xs">{item.externalKey}</p>
                        <p className="break-all text-[13px] text-muted-foreground">
                          {item.repository ?? "Team repository"}
                        </p>
                        {item.error ? <p className="break-words text-[13px] text-destructive">{item.error}</p> : null}
                      </div>
                      <ExecutionStatusBadge status={item.status} />
                      <div className="flex shrink-0 items-center gap-1.5">
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={item.status === "running" || queueBusy}
                          onClick={() => void runQueueItem(item)}
                        >
                          <Play /> Run
                        </Button>
                        {item.status === "failed" || item.status === "cancelled" || item.status === "running" ? (
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <Button size="icon-sm" variant="ghost" aria-label={`More actions for ${item.externalKey}`}>
                                <Ellipsis />
                              </Button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end">
                              {item.status === "failed" || item.status === "cancelled" ? (
                                <>
                                  <DropdownMenuItem disabled={queueBusy} onSelect={() => void retryQueueItem(item)}>
                                    Retry
                                  </DropdownMenuItem>
                                  <DropdownMenuItem disabled={queueBusy} onSelect={() => void requeueQueueItem(item)}>
                                    Requeue
                                  </DropdownMenuItem>
                                </>
                              ) : null}
                              {item.status === "running" ? (
                                <DropdownMenuItem
                                  destructive
                                  disabled={queueBusy}
                                  onSelect={() => void cancelQueueItem(item)}
                                >
                                  Cancel item
                                </DropdownMenuItem>
                              ) : null}
                            </DropdownMenuContent>
                          </DropdownMenu>
                        ) : null}
                      </div>
                    </div>
                  ))}
                </Card>
              )}
            </Section>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(detail)} onOpenChange={(next) => !next && setDetail(null)}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle className="break-words">{detailTask || "Agent team run"}</DialogTitle>
            <DialogDescription>
              {detail ? `${teamName(detail.run.teamId) ?? "Agent team"}, started ${relativeTime(detail.run.createdAt)}` : null}
            </DialogDescription>
          </DialogHeader>
          {detail ? (
            <div className="min-w-0 space-y-4">
              <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                <ExecutionStatusBadge status={detail.run.status} />
                <p className="tabular text-sm text-muted-foreground">
                  {detailDone} of {detail.steps.length} {detail.steps.length === 1 ? "step" : "steps"} done
                </p>
              </div>
              <div
                role="progressbar"
                aria-label="Steps done"
                aria-valuemin={0}
                aria-valuemax={detail.steps.length}
                aria-valuenow={detailDone}
                className="h-1.5 overflow-hidden rounded-full bg-muted"
              >
                <div
                  className="h-full bg-primary transition-[width]"
                  style={{ width: `${detail.steps.length ? (detailDone / detail.steps.length) * 100 : 0}%` }}
                />
              </div>
              {detail.run.error ? (
                <Callout tone="danger" title="The run failed">
                  {detail.run.error}
                </Callout>
              ) : null}
              <ol className="divide-y rounded-lg border">
                {detail.steps.map((step) => (
                  <li
                    key={step.id}
                    className="flex flex-col gap-2 px-3 py-3 sm:flex-row sm:items-center sm:gap-3"
                  >
                    <div className="flex min-w-0 flex-1 items-start gap-3">
                      <StepNumber n={step.stepIndex + 1} className="mt-0.5" />
                      <div className="min-w-0 space-y-0.5">
                        <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
                          <span className="min-w-0 break-words">{step.name}</span>
                          {step.stepIndex === 0 ? <Badge>Primary</Badge> : null}
                        </p>
                        <p className="text-[13px] text-muted-foreground">
                          {step.skills.length ? `Skills: ${step.skills.join(", ")}` : "No team skills"}
                        </p>
                        {step.error ? <p className="break-words text-[13px] text-destructive">{step.error}</p> : null}
                      </div>
                    </div>
                    <div className="flex items-center gap-3 pl-9 sm:pl-0">
                      <ExecutionStatusBadge status={step.status} />
                      {step.executionId ? (
                        <Button asChild size="sm" variant="outline">
                          <Link href={`/executions/${step.executionId}`}>Open execution</Link>
                        </Button>
                      ) : null}
                    </div>
                  </li>
                ))}
              </ol>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}
