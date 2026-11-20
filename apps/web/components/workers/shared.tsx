import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { StatusMeta } from "@/components/ui/status";
import { titleCase } from "@/lib/utils";

export const ACTIVE_STATUSES = ["pending", "starting", "running"];

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
export function launchErrorText(error: string): string {
  if (/worker launcher request failed|launcher.*error sending request|no worker launcher/i.test(error)) {
    return "ORBIT could not reach a worker launcher, so nothing was started. Start this worker with Bootstrap, or configure the launcher.";
  }
  return error;
}

export function launchStatusMeta(status: string): StatusMeta {
  return LAUNCH_STATUS[status] ?? { label: titleCase(status), tone: "neutral", shape: "ring" };
}

export function targetLabel(target: string) {
  return TARGET_LABEL[target] ?? titleCase(target);
}

export function isActive(launch: { status: string }) {
  return ACTIVE_STATUSES.includes(launch.status);
}

/** A group heading inside a long form. */
export function FormGroup({
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
export function PickList({
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
