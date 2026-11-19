import { ChoiceCard } from "@/components/ui/choice-card";

import { PickList } from "./shared";
import type { WorkerFormApi } from "./worker-form";

type Named = { id: string; name: string };

/** Which agents the worker may claim. */
export function AgentScopeFields({ form, agents }: { form: WorkerFormApi; agents: Named[] }) {
  const { values, set } = form;
  return (
    <>
      <div role="radiogroup" aria-label="Agent scope" className="grid gap-2 sm:grid-cols-2">
        <ChoiceCard
          selected={values.agentScope === "all"}
          onSelect={() => set({ agentScope: "all" })}
          title="All agents"
          description="Agents created later are included automatically."
        />
        <ChoiceCard
          selected={values.agentScope === "selected"}
          onSelect={() => set({ agentScope: "selected" })}
          title="Selected agents"
          description="Only the agents you pick."
        />
      </div>
      {values.agentScope === "selected" ? (
        <PickList
          label="agents"
          idPrefix="wp-agent"
          selected={values.agentIds}
          onChange={(agentIds) => set({ agentIds })}
          items={agents.map((agent) => ({ id: agent.id, label: agent.name }))}
          empty="There are no agents to select yet."
        />
      ) : null}
    </>
  );
}

/** Shared skills the worker prepares at startup. */
export function SkillFields({
  form,
  skills,
}: {
  form: WorkerFormApi;
  skills: Array<{ slug: string; name: string }>;
}) {
  return (
    <PickList
      label="skills"
      idPrefix="wp-skill"
      selected={form.values.skillSlugs}
      onChange={(skillSlugs) => form.set({ skillSlugs })}
      items={skills.map((skill) => ({ id: skill.slug, label: skill.name }))}
      empty="There are no skills to select yet."
    />
  );
}
