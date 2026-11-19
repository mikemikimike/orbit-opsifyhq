import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { DEFAULT_WORKER_IMAGES } from "@/lib/worker-images";

import { resolveProvider, type ProviderRow } from "./model-section";
import type { WorkerFormApi } from "./worker-form";

const DEFAULT_IMAGES = DEFAULT_WORKER_IMAGES;

/** Name, harness and target. */
export function BasicsFields({ form, providers }: { form: WorkerFormApi; providers: ProviderRow[] }) {
  const { values, set } = form;
  const { selectedProvider } = resolveProvider(providers, values.provider);
  return (
    <>
      <Field label="Name" htmlFor="wp-name">
        <Input id="wp-name" value={values.name} onChange={(event) => set({ name: event.target.value })} />
      </Field>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Harness" htmlFor="wp-harness">
          <Select
            id="wp-harness"
            value={values.harness}
            onChange={(event) => {
              const nextHarness = event.target.value;
              set({ harness: nextHarness, image: DEFAULT_IMAGES[nextHarness] ?? "" });
              const currentConfig = (selectedProvider?.config ?? {}) as Record<string, unknown>;
              if (selectedProvider && String(currentConfig.harness ?? "") !== nextHarness) {
                set({ provider: "", model: "" });
              }
              // Auto-suggest auth secret for harness-managed providers.
              if (nextHarness === "opencode" && !values.authSecret) {
                set({ authSecret: "orbit-opencode-auth" });
              } else if (nextHarness === "pi" && !values.authSecret) {
                set({ authSecret: "orbit-pi-auth" });
              } else if (nextHarness === "mock") {
                set({ authSecret: "" });
              }
            }}
          >
            <option value="pi">Pi</option>
            <option value="opencode">OpenCode</option>
            <option value="mock">Mock</option>
          </Select>
        </Field>
        <Field label="Target" htmlFor="wp-target" hint="Where the worker runs.">
          <Select id="wp-target" value={values.target} onChange={(event) => set({ target: event.target.value })}>
            <option value="docker">Docker</option>
            <option value="kubernetes">Kubernetes</option>
            <option value="local">Local</option>
          </Select>
        </Field>
      </div>
    </>
  );
}

/** Execution driver, image and tag. */
export function ImageFields({
  form,
  platformVersion,
}: {
  form: WorkerFormApi;
  platformVersion: string | undefined;
}) {
  const { values, set } = form;
  const { tag } = values;
  return (
    <>
      <Field
        label="Execution driver"
        htmlFor="wp-driver"
        hint="A long-lived process claims many executions. Docker starts a container for each one."
      >
        <Select id="wp-driver" value={values.driver} onChange={(event) => set({ driver: event.target.value })}>
          <option value="process">Process (long-lived worker)</option>
          <option value="docker">Docker (container per execution)</option>
        </Select>
      </Field>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Image" htmlFor="wp-image">
          <Input
            id="wp-image"
            className="font-mono text-[13px]"
            value={values.image}
            onChange={(event) => set({ image: event.target.value })}
          />
        </Field>
        <Field
          label="Tag"
          htmlFor="wp-tag"
          hint={
            <>
              Leave empty to follow the platform&apos;s version; the worker is then relaunched
              automatically after an upgrade.
              {platformVersion && tag && tag !== platformVersion ? (
                <>
                  {" "}
                  <button
                    type="button"
                    className="text-primary underline-offset-2 hover:underline"
                    onClick={() => set({ tag: "" })}
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
            onChange={(event) => set({ tag: event.target.value })}
          />
        </Field>
      </div>
    </>
  );
}
