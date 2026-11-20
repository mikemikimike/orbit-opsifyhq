"use client";

export const dynamic = "force-dynamic";

import { useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Play, Plus, Search, Users, X } from "lucide-react";

import { ImportPortableButton } from "@/components/import-portable-dialog";
import { AgentLineup, MODE_LABEL, agentLookup, teamMode } from "@/components/teams/team-parts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Callout } from "@/components/ui/callout";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { Segmented } from "@/components/ui/segmented";
import { Skeleton } from "@/components/ui/skeleton";
import { ExecutionStatusBadge } from "@/components/ui/status-badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAllAgents, useAgentTeams, useTeamRuns } from "@/lib/queries";
import type { AgentTeam, TeamRun } from "@/lib/types";
import { relativeTime } from "@/lib/utils";
import { errorMessage } from "@orbit/api-client";

type ListTab = "teams" | "runs";

function TeamRow({ team, agentName }: { team: AgentTeam; agentName: (id: string) => string }) {
  const mode = teamMode(team);
  const source = team.definition.workSource;
  return (
    <li className="group relative border-b last:border-b-0">
      <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-2 px-4 py-3.5 transition-colors group-hover:bg-muted/50 sm:px-5 md:grid-cols-[minmax(0,1fr)_minmax(0,16rem)_6.5rem]">
        <div className="min-w-0">
          <p className="flex min-w-0 items-center gap-2">
            <Link
              href={`/agent-teams/${team.id}`}
              className="truncate text-sm font-semibold after:absolute after:inset-0 focus-visible:outline-none focus-visible:after:outline focus-visible:after:outline-2 focus-visible:after:-outline-offset-2 focus-visible:after:outline-ring"
            >
              {team.name}
            </Link>
            {source?.enabled ? (
              <Badge variant={team.lastPollError ? "warning" : "secondary"} className="shrink-0">
                {team.lastPollError ? "Work source failing" : "Picks up issues"}
              </Badge>
            ) : null}
          </p>
          <p className="mt-0.5 line-clamp-1 text-[13px] text-muted-foreground">
            {team.description || team.definition.instructions || "No description"}
          </p>
          <div className="mt-1.5 text-xs text-muted-foreground md:hidden">
            <AgentLineup team={team} agentName={agentName} />
          </div>
        </div>

        <div className="hidden min-w-0 text-[13px] md:block">
          <AgentLineup team={team} agentName={agentName} className="text-muted-foreground" />
          <p className="mt-0.5 truncate text-xs text-muted-foreground">
            {MODE_LABEL[mode]}
            {team.definition.repository ? <span className="font-mono"> · {team.definition.repository}</span> : null}
          </p>
        </div>

        <p className="tabular text-right text-[13px] text-muted-foreground">{relativeTime(team.updatedAt)}</p>
      </div>
    </li>
  );
}

function RunRow({ run, teamName }: { run: TeamRun; teamName?: string }) {
  const task = String(run.input.task ?? "") || "Agent team run";
  return (
    <li className="group relative border-b last:border-b-0">
      <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1.5 px-4 py-3.5 transition-colors group-hover:bg-muted/50 sm:px-5 md:grid-cols-[8rem_minmax(0,1fr)_6.5rem]">
        <ExecutionStatusBadge status={run.status} className="col-start-2 row-start-1 justify-self-end md:col-start-auto md:justify-self-start" />
        <div className="col-start-1 row-start-1 min-w-0 md:col-start-auto">
          <Link
            href={`/agent-teams/runs/${run.id}`}
            className="block truncate text-sm font-medium after:absolute after:inset-0 focus-visible:outline-none focus-visible:after:outline focus-visible:after:outline-2 focus-visible:after:-outline-offset-2 focus-visible:after:outline-ring"
          >
            {task}
          </Link>
          <p className="truncate text-[13px] text-muted-foreground">{teamName ?? "Agent team"}</p>
          {run.error ? <p className="line-clamp-1 text-[13px] text-destructive">{run.error}</p> : null}
        </div>
        <p className="tabular hidden text-right text-[13px] text-muted-foreground md:block">{relativeTime(run.createdAt)}</p>
      </div>
    </li>
  );
}

function RowsSkeleton() {
  return (
    <Card className="overflow-hidden">
      {Array.from({ length: 3 }).map((_, index) => (
        <div key={index} className="border-b px-5 py-4 last:border-b-0">
          <Skeleton className="h-10" />
        </div>
      ))}
    </Card>
  );
}

export default function AgentTeamsPage() {
  const router = useRouter();
  const teams = useAgentTeams();
  const agents = useAllAgents();
  const [runsFilter, setRunsFilter] = useState<"active" | "archived" | "all">("active");
  const runs = useTeamRuns(runsFilter === "all" ? undefined : runsFilter === "archived");
  const [tab, setTab] = useState<ListTab>("teams");
  const [search, setSearch] = useState("");

  // Old links (`/agent-teams?run=<id>`) open the run page; `#runs` opens the Runs tab.
  useEffect(() => {
    const runId = new URLSearchParams(window.location.search).get("run");
    if (runId) {
      router.replace(`/agent-teams/runs/${runId}`);
      return;
    }
    if (window.location.hash === "#runs") setTab("runs");
  }, [router]);

  function selectTab(next: string) {
    setTab(next as ListTab);
    window.history.replaceState(null, "", next === "teams" ? window.location.pathname : `#${next}`);
  }

  const agent = useMemo(() => agentLookup(agents.data?.items ?? []), [agents.data]);
  const agentName = (id: string) => agent(id).name;
  const teamName = (id: string) => teams.data?.find((team) => team.id === id)?.name;

  const visibleTeams = useMemo(() => {
    const terms = search.toLowerCase().split(/\s+/).filter(Boolean);
    return (teams.data ?? []).filter((team) => {
      if (!terms.length) return true;
      const names = (team.definition.steps ?? []).map((step) => agent(step.agentId).name).join(" ");
      const haystack = `${team.name} ${team.description} ${team.definition.repository ?? ""} ${names}`.toLowerCase();
      return terms.every((term) => haystack.includes(term));
    });
  }, [teams.data, search, agent]);

  return (
    <div className="space-y-6">
      <PageHeader
        title="Agent teams"
        description="A primary agent and its helpers working toward one goal. Open a team to run it, follow its runs and manage its queue."
        actions={
          <div className="flex flex-wrap gap-2">
            <ImportPortableButton />
            <Button asChild>
              <Link href="/agent-teams/new">
                <Plus /> New agent team
              </Link>
            </Button>
          </div>
        }
      />

      <Tabs value={tab} onValueChange={selectTab}>
        <TabsList>
          <TabsTrigger value="teams">
            Teams
            {teams.data ? <span className="tabular text-xs text-muted-foreground">{teams.data.length}</span> : null}
          </TabsTrigger>
          <TabsTrigger value="runs">Runs</TabsTrigger>
        </TabsList>

        <TabsContent value="teams" className="space-y-4">
          {(teams.data?.length ?? 0) > 3 ? (
            <div className="relative md:w-80">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="search"
                className="pl-9 pr-9 [&::-webkit-search-cancel-button]:hidden"
                placeholder="Filter by name, agent or repository"
                aria-label="Filter agent teams"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
              {search ? (
                <button
                  type="button"
                  aria-label="Clear filter"
                  onClick={() => setSearch("")}
                  className="absolute right-1.5 top-1/2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              ) : null}
            </div>
          ) : null}

          {teams.isLoading ? (
            <RowsSkeleton />
          ) : teams.isError ? (
            <Callout tone="danger" title="Agent teams could not be loaded">
              {errorMessage(teams.error)}
            </Callout>
          ) : !teams.data?.length ? (
            <EmptyState
              icon={<Users />}
              title="No agent teams yet"
              description="A team pairs a primary agent with helpers, such as a builder, a tester and a reviewer, that take over in turn or work as its subagents."
              action={
                <Button asChild variant="outline">
                  <Link href="/agent-teams/new">
                    <Plus /> New agent team
                  </Link>
                </Button>
              }
            />
          ) : visibleTeams.length ? (
            <Card className="overflow-hidden">
              <div
                aria-hidden
                className="hidden grid-cols-[minmax(0,1fr)_minmax(0,16rem)_6.5rem] gap-x-4 border-b bg-subtle px-5 py-2 text-xs font-medium text-muted-foreground md:grid"
              >
                <span>Team</span>
                <span>Agents</span>
                <span className="text-right">Updated</span>
              </div>
              <ul>
                {visibleTeams.map((team) => (
                  <TeamRow key={team.id} team={team} agentName={agentName} />
                ))}
              </ul>
            </Card>
          ) : (
            <EmptyState
              icon={<Search />}
              title="No teams match"
              description={`Nothing matches "${search}".`}
              action={
                <Button variant="outline" onClick={() => setSearch("")}>
                  Clear filter
                </Button>
              }
            />
          )}
        </TabsContent>

        <TabsContent value="runs" className="space-y-4">
          <Segmented
            label="Show runs"
            value={runsFilter}
            onChange={setRunsFilter}
            options={[
              { value: "active", label: "Active" },
              { value: "archived", label: "Archived" },
              { value: "all", label: "All" },
            ]}
          />
          {runs.isLoading ? (
            <RowsSkeleton />
          ) : runs.isError ? (
            <Callout tone="danger" title="Runs could not be loaded">
              {errorMessage(runs.error)}
            </Callout>
          ) : !runs.data?.length ? (
            <EmptyState
              icon={<Play />}
              title={runsFilter === "active" ? "No runs yet" : `No ${runsFilter === "all" ? "" : "archived "}runs`}
              description={
                runsFilter === "active"
                  ? (teams.data ?? []).length
                    ? "Open a team and choose Run. Its runs show up here."
                    : "Create a team first; its runs show up here."
                  : "Nothing matches this filter."
              }
              action={
                runsFilter !== "active" ? (
                  <Button variant="outline" onClick={() => setRunsFilter("active")}>
                    Show active runs
                  </Button>
                ) : undefined
              }
            />
          ) : (
            <Card className="overflow-hidden">
              <div
                aria-hidden
                className="hidden grid-cols-[8rem_minmax(0,1fr)_6.5rem] gap-x-4 border-b bg-subtle px-5 py-2 text-xs font-medium text-muted-foreground md:grid"
              >
                <span>Status</span>
                <span>Task</span>
                <span className="text-right">Started</span>
              </div>
              <ul>
                {runs.data.map((run) => (
                  <RunRow key={run.id} run={run} teamName={teamName(run.teamId)} />
                ))}
              </ul>
            </Card>
          )}
        </TabsContent>
      </Tabs>
    </div>
  );
}
