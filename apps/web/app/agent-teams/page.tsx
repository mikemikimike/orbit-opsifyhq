"use client";

import { useEffect, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Pencil, Plus, Play, Trash2, Users, X } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { PageHeader } from "@/components/ui/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import {
  queryKeys,
  useAgents,
  useAgentTeams,
  useSkills,
  useTeamRuns,
} from "@/lib/queries";
import { errorMessage } from "@orbit/api-client";
import {
  createAgentTeam,
  deleteAgentTeam,
  getTeamRun,
  startTeamRun,
  updateAgentTeam,
  type TeamStepInput,
} from "@/lib/teams";
import type { AgentTeam, TeamRunDetail } from "@/lib/types";
import { relativeTime } from "@/lib/utils";

export default function AgentTeamsPage() {
  const teams = useAgentTeams();
  const runs = useTeamRuns();
  const agents = useAgents({ limit: 100 });
  const skills = useSkills();
  const queryClient = useQueryClient();

  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<AgentTeam | null>(null);
  const [mode, setMode] = useState<"pipeline" | "native">("native");
  const [name, setName] = useState("Coding team");
  const [description, setDescription] = useState(
    "Primary coding agent with specialist agents for implementation and review.",
  );
  const [instructions, setInstructions] = useState(
    "Pick up the next GitHub issue labelled agent-ready, implement it, test it and request review. Repeat until no ready issues remain.",
  );
  const [teamSkills, setTeamSkills] = useState<string[]>([]);
  const [steps, setSteps] = useState<TeamStepInput[]>([
    { agentId: "" },
    { agentId: "" },
  ]);
  const [runTeam, setRunTeam] = useState<AgentTeam | null>(null);
  const [task, setTask] = useState("");
  const [repository, setRepository] = useState("");
  const [detail, setDetail] = useState<TeamRunDetail | null>(null);

  const agentOptions = agents.data?.items ?? [];
  const skillOptions = skills.data ?? [];
  const canCreate =
    name.trim() &&
    steps.length > 0 &&
    steps.every((step) => step.agentId) &&
    new Set(steps.map((step) => step.agentId)).size === steps.length;

  const agentLabel = useMemo(() => {
    const map = new Map(
      agentOptions.map((agent) => [agent.id, `${agent.name} · ${agent.harness}`]),
    );
    return (id: string) => map.get(id) ?? id.slice(0, 8);
  }, [agentOptions]);

  const save = useMutation({
    mutationFn: () => {
      const payload = {
        name: name.trim(),
        description,
        mode,
        instructions,
        skills: teamSkills,
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
    setDescription("Primary coding agent with specialist agents for implementation and review.");
    setInstructions(
      "Pick up the next GitHub issue labelled agent-ready, implement it, test it and request review. Repeat until no ready issues remain.",
    );
    setTeamSkills([]);
    setSteps([{ agentId: "" }, { agentId: "" }]);
    setOpen(true);
  }

  function openEdit(team: AgentTeam) {
    setEditing(team);
    setMode(team.definition.mode ?? "pipeline");
    setName(team.name);
    setDescription(team.description);
    setInstructions(team.definition.instructions ?? "");
    setTeamSkills(team.definition.skills ?? []);
    setSteps((team.definition.steps ?? []).map((step) => ({ agentId: step.agentId })));
    setOpen(true);
  }

  const remove = useMutation({
    mutationFn: deleteAgentTeam,
    onSuccess: () => {
      toast.success("Agent team deleted");
      void queryClient.invalidateQueries({ queryKey: queryKeys.agentTeams });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const run = useMutation({
    mutationFn: () => startTeamRun(runTeam!.id, task.trim(), repository),
    onSuccess: (result) => {
      toast.success("Agent team run started");
      setRunTeam(null);
      setDetail(result);
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
    if (runId) void inspect(runId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="space-y-5 animate-fade-in">
      <PageHeader
        eyebrow="Orchestration"
        title="Agent Teams"
        description="Ordered primary and specialist agents. Every step is a normal execution with its own connector grants and approvals; skills and the team goal are shared."
        actions={
          <Button onClick={openCreate}>
            <Plus /> New agent team
          </Button>
        }
      />

      {teams.isLoading ? (
        <Skeleton className="h-40 rounded-xl" />
      ) : teams.isError ? (
        <p className="text-sm text-destructive">{errorMessage(teams.error)}</p>
      ) : !teams.data?.length ? (
        <EmptyState
          icon={<Users className="h-6 w-6" />}
          title="No agent teams"
          description="Create a primary coding agent followed by builder, tester and reviewer agents."
          action={<Button onClick={openCreate}><Plus /> Create agent team</Button>}
        />
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {teams.data.map((team) => {
            const steps = team.definition.steps ?? [];
            return (
              <Card key={team.id}>
                <CardHeader className="pb-2">
                  <CardTitle className="truncate">{team.name}</CardTitle>
                  <p className="text-[11px] text-muted-foreground">
                    {(team.definition.mode ?? "pipeline") === "native" ? "native subagents" : "pipeline"} ·{" "}
                    {steps.length} agents · {team.definition.skills?.length ?? 0} skills · updated{" "}
                    {relativeTime(team.updatedAt)}
                  </p>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div className="flex flex-wrap gap-1">
                    {steps.map((step, index) => (
                      <Badge key={`${step.agentId}-${index}`} variant={index === 0 ? "default" : "secondary"}>
                        {index === 0 ? "Primary · " : "Subagent · "}
                        {agentLabel(step.agentId)}
                      </Badge>
                    ))}
                  </div>
                  {team.definition.skills?.length ? (
                    <p className="text-[11px] text-muted-foreground">
                      team skills: {team.definition.skills.join(", ")}
                    </p>
                  ) : null}
                  <div className="flex flex-wrap gap-2">
                    <Button
                      size="sm"
                      onClick={() => {
                        setRunTeam(team);
                        setTask(team.definition.instructions ?? "");
                        setRepository("");
                      }}
                    >
                      <Play /> Run
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => openEdit(team)}
                    >
                      <Pencil /> Edit
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-destructive"
                      onClick={() => {
                        if (window.confirm(`Delete agent team ${team.name}?`)) remove.mutate(team.id);
                      }}
                    >
                      <Trash2 /> Delete
                    </Button>
                  </div>
                </CardContent>
              </Card>
            );
          })}
        </div>
      )}

      <div className="space-y-3">
        <div>
          <h2 className="text-sm font-semibold">Agent team runs</h2>
          <p className="text-[11px] text-muted-foreground">
            Sequential handoffs; each step is recorded as a normal execution.
          </p>
        </div>
        {runs.isLoading ? (
          <Skeleton className="h-20 rounded-xl" />
        ) : !runs.data?.length ? (
          <p className="rounded-xl border border-dashed p-4 text-xs text-muted-foreground">
            No agent team runs yet.
          </p>
        ) : (
          <div className="space-y-2">
            {runs.data.map((item) => (
              <Card key={item.id}>
                <CardContent className="flex flex-wrap items-center justify-between gap-3 p-4">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">
                      {String(item.input.task ?? "Agent team run")}
                    </p>
                    <p className="text-[11px] text-muted-foreground">
                      {relativeTime(item.createdAt)} · {item.status}
                    </p>
                    {item.error ? <p className="text-[11px] text-destructive">{item.error}</p> : null}
                  </div>
                  <div className="flex items-center gap-2">
                    <Badge
                      variant={
                        item.status === "succeeded"
                          ? "success"
                          : item.status === "failed"
                            ? "destructive"
                            : "warning"
                      }
                    >
                      {item.status}
                    </Badge>
                    <Button size="sm" variant="outline" onClick={() => void inspect(item.id)}>
                      <ChevronRight /> Steps
                    </Button>
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        )}
      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{editing ? "Edit agent team" : "New agent team"}</DialogTitle>
            <DialogDescription>
              The first agent is the primary agent. Its agent instructions define its role;
              the team instruction defines the shared goal.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label>Name</Label>
                <Input value={name} onChange={(event) => setName(event.target.value)} />
              </div>
              <div className="space-y-1.5">
                <Label>Description</Label>
                <Input value={description} onChange={(event) => setDescription(event.target.value)} />
              </div>
            </div>
            <div className="space-y-1.5">
              <Label>Execution mode</Label>
              <select
                className="flex h-9 w-full rounded-lg border border-input bg-card px-3 text-sm"
                value={mode}
                onChange={(event) => setMode(event.target.value as "pipeline" | "native")}
              >
                <option value="native">Native subagents — one OpenCode execution</option>
                <option value="pipeline">Pipeline — one execution per agent</option>
              </select>
              <p className="text-[11px] text-muted-foreground">
                Native mode runs the primary agent once and exposes later agents as OpenCode
                subagents. Pipeline mode gives every agent a separate ORBIT execution.
              </p>
            </div>
            <div className="space-y-1.5">
              <Label>Team instruction / goal</Label>
              <Textarea
                className="min-h-20"
                value={instructions}
                onChange={(event) => setInstructions(event.target.value)}
                placeholder="Pick up issues labelled agent-ready, fix them in a loop, and stop when none remain."
              />
              <p className="text-[11px] text-muted-foreground">
                Passed to the primary agent and included in every later step with the previous output.
              </p>
            </div>
            <div className="space-y-1.5">
              <Label>Team skills</Label>
              <div className="flex flex-wrap gap-2 rounded-lg border p-3">
                {skillOptions.length ? skillOptions.map((skill) => {
                  const checked = teamSkills.includes(skill.slug);
                  return (
                    <label key={skill.slug} className="flex items-center gap-1 text-xs">
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={(event) =>
                          setTeamSkills((current) =>
                            event.target.checked
                              ? [...current, skill.slug]
                              : current.filter((slug) => slug !== skill.slug),
                          )
                        }
                      />
                      {skill.name}
                    </label>
                  );
                }) : <span className="text-[11px] text-muted-foreground">No skills configured.</span>}
              </div>
            </div>
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>Agents in order</Label>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setSteps((current) => [...current, { agentId: "" }])}
                >
                  <Plus /> Add agent
                </Button>
              </div>
              {steps.map((step, index) => (
                <div key={index} className="flex items-center gap-2">
                  <span className="w-16 text-[11px] text-muted-foreground">
                    {index === 0 ? "Primary" : "Subagent"}
                  </span>
                  <select
                    className="flex h-9 flex-1 rounded-lg border border-input bg-card px-3 text-sm"
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
                        {agent.name} · {agent.harness}
                      </option>
                    ))}
                  </select>
                  <Button
                    type="button"
                    size="icon"
                    variant="ghost"
                    disabled={steps.length <= 1}
                    onClick={() => setSteps((current) => current.filter((_, i) => i !== index))}
                  >
                    <X />
                  </Button>
                </div>
              ))}
            </div>
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setOpen(false)}>Cancel</Button>
            <Button onClick={() => save.mutate()} loading={save.isPending} disabled={!canCreate}>
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
              The primary agent receives this task. Later agents receive the previous output.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label>Task for this run</Label>
            <Textarea
              className="min-h-24"
              value={task}
              onChange={(event) => setTask(event.target.value)}
              placeholder={runTeam?.definition.instructions || "Team goal"}
            />
            <Label>Repository</Label>
            <Input
              value={repository}
              onChange={(event) => setRepository(event.target.value)}
              placeholder="owner/repo (optional if the connection sets defaultRepository)"
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRunTeam(null)}>Cancel</Button>
            <Button
              onClick={() => run.mutate()}
              loading={run.isPending}
              disabled={!task.trim() && !runTeam?.definition.instructions}
            >
              Start run
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(detail)} onOpenChange={(next) => !next && setDetail(null)}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>Agent team run</DialogTitle>
            <DialogDescription>{detail?.run.status}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            {detail?.steps.map((step) => (
              <div key={step.id} className="flex flex-wrap items-center justify-between gap-2 rounded-lg border p-3">
                <div>
                  <p className="text-sm font-medium">
                    {step.stepIndex === 0 ? "Primary · " : "Subagent · "}
                    {step.name}
                  </p>
                  <p className="text-[11px] text-muted-foreground">
                    {step.skills.join(", ") || "no team skills"}
                  </p>
                  {step.error ? <p className="text-[11px] text-destructive">{step.error}</p> : null}
                </div>
                <div className="flex items-center gap-2">
                  <Badge variant={step.status === "succeeded" ? "success" : step.status === "failed" ? "destructive" : "warning"}>
                    {step.status}
                  </Badge>
                  {step.executionId ? (
                    <a className="text-xs text-primary hover:underline" href={`/executions/${step.executionId}`}>
                      Execution
                    </a>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
