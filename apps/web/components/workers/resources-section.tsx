import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";

import type { WorkerFormApi } from "./worker-form";

/** CPU, memory and workspace requests and limits for the worker container. */
export function ResourceFields({ form }: { form: WorkerFormApi }) {
  const { values, set } = form;
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <Field label="CPU request" htmlFor="wp-cpu-request">
        <Input
          id="wp-cpu-request"
          value={values.cpuRequest}
          onChange={(event) => set({ cpuRequest: event.target.value })}
          placeholder="100m"
        />
      </Field>
      <Field label="Memory request" htmlFor="wp-mem-request">
        <Input
          id="wp-mem-request"
          value={values.memoryRequest}
          onChange={(event) => set({ memoryRequest: event.target.value })}
          placeholder="256Mi"
        />
      </Field>
      <Field label="CPU limit" htmlFor="wp-cpu-limit">
        <Input
          id="wp-cpu-limit"
          value={values.cpuLimit}
          onChange={(event) => set({ cpuLimit: event.target.value })}
          placeholder="1"
        />
      </Field>
      <Field label="Memory limit" htmlFor="wp-mem-limit">
        <Input
          id="wp-mem-limit"
          value={values.memoryLimit}
          onChange={(event) => set({ memoryLimit: event.target.value })}
          placeholder="1Gi"
        />
      </Field>
      <Field label="Workspace size" htmlFor="wp-workspace">
        <Input
          id="wp-workspace"
          value={values.workspaceSize}
          onChange={(event) => set({ workspaceSize: event.target.value })}
          placeholder="1Gi"
        />
      </Field>
    </div>
  );
}
