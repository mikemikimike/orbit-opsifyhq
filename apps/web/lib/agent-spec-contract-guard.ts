/**
 * Compile-time guard (#109): the Zod `agentSpecSchema` strips unknown keys, and the agent
 * form submits the parsed result, so any API `AgentSpec` field missing from the schema is
 * silently dropped on every UI save. This file has no runtime logic; it fails `tsc` when a
 * path present in the generated API type is absent from the Zod output type.
 */
import type { components } from "@orbit/api-client";
import type { agentSpecSchema } from "@orbit/contracts";
import type { z } from "zod";

type Paths<T, P extends string = ""> = T extends readonly (infer U)[]
  ? Paths<U, P>
  : T extends object
    ? string extends keyof T
      ? never // open-ended record: keys are data, not schema
      : {
          [K in keyof T & string]-?: `${P}${K}` | Paths<NonNullable<T[K]>, `${P}${K}.`>;
        }[keyof T & string]
    : never;

type ApiPaths = Paths<components["schemas"]["AgentSpec"]>;
type ZodPaths = Paths<z.output<typeof agentSpecSchema>>;

/**
 * Known OpenAPI inaccuracy: `ConnectorBinding` serializes through a repr that renames the
 * field to `connectionId`, but utoipa documents it as `connection_id`.
 */
type KnownApiSchemaQuirks = "connectors.connection_id";

type MissingFromZod = Exclude<ApiPaths, ZodPaths | KnownApiSchemaQuirks>;

/**
 * If this errors, the error text names the missing path(s) (e.g. `"harnessConfig"`):
 * add them to `agentSpecSchema` in packages/contracts.
 */
type Assert<Missing> = [Missing] extends [never] ? "ok" : Missing;

export const agentSpecContractComplete: Assert<MissingFromZod> = "ok";
