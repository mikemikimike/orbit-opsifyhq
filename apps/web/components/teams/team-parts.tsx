import Link from "next/link";
import { ArrowRight } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import type { TeamModel } from "@/lib/teams";
import type { AgentSummary, AgentTeam } from "@/lib/types";
import { cn } from "@/lib/utils";

export type TeamMode = "pipeline" | "native";

export const MODE_LABEL: Record<TeamMode, string> = {
  native: "Native subagents",
  pipeline: "Pipeline",
};

export const MODE_HINT: Record<TeamMode, string> = {
  native: "One OpenCode execution. The primary agent runs once and the others are its subagents.",
  pipeline: "One execution per agent. Each agent takes over in turn and receives the previous output.",
};

export function teamMode(team: AgentTeam): TeamMode {
  return team.definition.mode ?? "pipeline";
}

/** "provider/model", "model" or null. */
export function modelLabel(model?: TeamModel | null): string | null {
  if (!model?.model) return null;
  return model.provider ? `${model.provider}/${model.model}` : model.model;
}

/** Look up agents by id, falling back to a short id when the agent is not visible. */
export function agentLookup(agents: AgentSummary[]) {
  const map = new Map(agents.map((agent) => [agent.id, agent]));
  return (id: string) => {
    const agent = map.get(id);
    return { id, name: agent?.name ?? id.slice(0, 8), harness: agent?.harness ?? null, known: Boolean(agent) };
  };
}

/** The position of an agent in the team. */
export function StepNumber({ n, className }: { n: number; className?: string }) {
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

/**
 * The agents of a team on one line: "Planner → Builder → Reviewer" for a pipeline,
 * "Lead with Builder, Tester" for native subagents. Used in lists, where the order is
 * the most useful thing to see at a glance.
 */
export function AgentLineup({
  team,
  agentName,
  className,
}: {
  team: AgentTeam;
  agentName: (id: string) => string;
  className?: string;
}) {
  const steps = team.definition.steps ?? [];
  if (!steps.length) {
    return <span className={cn("text-muted-foreground", className)}>No agents yet</span>;
  }
  const names = steps.map((step) => agentName(step.agentId));
  if (teamMode(team) === "native") {
    const [lead, ...rest] = names;
    return (
      <span className={cn("min-w-0 truncate", className)}>
        <span className="font-medium text-foreground">{lead}</span>
        {rest.length ? <span className="text-muted-foreground"> with {rest.join(", ")}</span> : null}
      </span>
    );
  }
  return (
    <span className={cn("flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5", className)}>
      {names.map((name, index) => (
        <span key={`${name}-${index}`} className="inline-flex min-w-0 items-center gap-1.5">
          {index > 0 ? <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" aria-label="then" /> : null}
          <span className={cn("truncate", index === 0 && "font-medium text-foreground")}>{name}</span>
        </span>
      ))}
    </span>
  );
}

/** The ordered agent list on the team page. */
export function AgentSteps({
  team,
  agent,
}: {
  team: AgentTeam;
  agent: (id: string) => { id: string; name: string; harness: string | null; known: boolean };
}) {
  const steps = team.definition.steps ?? [];
  const mode = teamMode(team);
  if (!steps.length) {
    return <p className="text-sm text-muted-foreground">No agents yet. Edit the team to add some.</p>;
  }
  return (
    <ol className="relative space-y-3">
      {steps.map((step, index) => {
        const info = agent(step.agentId);
        const model = modelLabel(step.model);
        return (
          <li key={`${step.agentId}-${index}`} className="flex items-start gap-3">
            <StepNumber n={index + 1} className="mt-px" />
            <div className="min-w-0 flex-1">
              <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
                {info.known ? (
                  <Link href={`/agents/${info.id}`} className="min-w-0 truncate text-sm font-medium hover:text-primary hover:underline">
                    {info.name}
                  </Link>
                ) : (
                  <span className="min-w-0 truncate text-sm font-medium">{info.name}</span>
                )}
                {index === 0 ? <Badge>Primary</Badge> : mode === "native" ? <Badge variant="secondary">Subagent</Badge> : null}
              </p>
              <p className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
                {info.harness ? <span className="font-mono">{info.harness}</span> : <span>Harness per run</span>}
                {model ? <span className="font-mono">model {model}</span> : null}
                {index > 0 && mode === "pipeline" ? <span>Receives step {index}&apos;s output</span> : null}
              </p>
            </div>
          </li>
        );
      })}
    </ol>
  );
}
