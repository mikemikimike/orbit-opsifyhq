import type { components } from "@orbit/api-client";

export type AgentSummary = components["schemas"]["AgentSummary"];
export type AgentDetail = components["schemas"]["AgentDetail"];
export type AgentVersionView = components["schemas"]["AgentVersionView"];
export type ExecutionRecord = components["schemas"]["ExecutionRecord"];
export type ExecutionEventRecord = components["schemas"]["ExecutionEventRecord"];
export type HarnessView = components["schemas"]["HarnessView"];
export type ProviderView = components["schemas"]["ProviderView"];
export type ConnectorView = components["schemas"]["ConnectorView"];
export type ConnectionView = components["schemas"]["ConnectionView"];
export type TriggerRecord = components["schemas"]["Trigger"];
export type DashboardResponse = components["schemas"]["DashboardResponse"];
export type MeResponse = components["schemas"]["MeResponse"];
export type MetaResponse = components["schemas"]["MetaResponse"];
export type AgentPage = components["schemas"]["AgentPage"];
export type ExecutionPage = components["schemas"]["ExecutionPage"];
export type ApiTokenView = components["schemas"]["ApiTokenView"];
export type CreatedApiToken = components["schemas"]["CreatedApiToken"];
export type Organization = components["schemas"]["Organization"];
export type User = components["schemas"]["User"];
export type AgentDefinition = components["schemas"]["AgentDefinition"];
export type AgentSpec = components["schemas"]["AgentSpec"];
export type AgentInstanceView = components["schemas"]["AgentInstanceView"];
export type ProviderProbeResponse = components["schemas"]["ProviderProbeResponse"];
export type ApprovalWithExecution = components["schemas"]["ApprovalWithExecution"];
export type WorkerView = components["schemas"]["WorkerView"];
export type Skill = components["schemas"]["Skill"];
export type WorkerProfile = components["schemas"]["WorkerProfile"];
export type WorkerBootstrap = components["schemas"]["WorkerBootstrap"];
// Generated from the API schema so the UI cannot drift from the server contract.
export type WorkSource = components["schemas"]["WorkSourceDefinition"] & {
  provider: "github" | "gitlab";
};
export type TeamRepository = components["schemas"]["RepositorySpec"];

export type AgentTeam = {
  id: string;
  orgId: string;
  name: string;
  description: string;
  definition: {
    mode?: "pipeline" | "native";
    instructions?: string;
    repository?: string | null;
    branch?: string | null;
    workSource?: WorkSource | null;
    repositories?: TeamRepository[];
    model?: { provider?: string | null; model?: string | null; management?: string | null } | null;
    workerProfileId?: string | null;
    autoLaunch?: boolean;
    skills?: string[];
    harness?: string | null;
    connectionIds?: string[];
    envMappingIds?: string[];
    timeoutSeconds?: number | null;
    steps?: Array<{ agentId: string; model?: { provider?: string | null; model?: string | null; management?: string | null } | null }>;
  };
  createdBy?: string | null;
  createdAt: string;
  updatedAt: string;
  lastPollAt?: string | null;
  lastPollError?: string | null;
};

export type TeamRun = {
  id: string;
  orgId: string;
  teamId: string;
  status: string;
  input: Record<string, unknown>;
  output?: unknown;
  error?: string | null;
  requestedBy?: string | null;
  createdAt: string;
  updatedAt: string;
  finishedAt?: string | null;
};

export type TeamStep = {
  id: string;
  runId: string;
  orgId: string;
  stepIndex: number;
  name: string;
  agentId: string;
  agentVersionId: string;
  skills: string[];
  status: string;
  executionId?: string | null;
  input: Record<string, unknown>;
  output?: unknown;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
  startedAt?: string | null;
  finishedAt?: string | null;
};

export type TeamRunDetail = { run: TeamRun; steps: TeamStep[] };

export type WorkerLaunch = {
  id: string;
  orgId: string;
  profileId: string;
  name: string;
  target: string;
  status: string;
  runtimeId?: string | null;
  image: string;
  workerId?: string | null;
  workerTokenId?: string | null;
  error?: string | null;
  createdBy?: string | null;
  createdAt: string;
  updatedAt: string;
  startedAt?: string | null;
  stoppedAt?: string | null;
};
