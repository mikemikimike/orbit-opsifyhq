"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/** The last lines of a launch's runtime output. */
export function LogsDialog({
  open,
  onOpenChange,
  name,
  logs,
  loading,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  name: string | undefined;
  logs: string;
  loading: boolean;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Logs for {name}</DialogTitle>
          <DialogDescription>The last 200 lines from the launcher runtime.</DialogDescription>
        </DialogHeader>
        <pre className="code-block max-h-[60vh] whitespace-pre-wrap break-all" aria-busy={loading}>
          {loading ? "Loading…" : logs}
        </pre>
      </DialogContent>
    </Dialog>
  );
}

/** Shown when a save changes something a running worker only picks up after a restart. */
export function RelaunchOnSaveDialog({
  open,
  onOpenChange,
  launchCount,
  onSaveOnly,
  onSaveAndRelaunch,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  launchCount: number;
  onSaveOnly: () => void;
  onSaveAndRelaunch: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Relaunch the worker now?</DialogTitle>
          <DialogDescription>
            This profile has a running worker and your changes need a restart to apply. Agents
            running on it may be interrupted.
          </DialogDescription>
        </DialogHeader>
        <p className="text-[13px] text-muted-foreground">
          {launchCount} {launchCount === 1 ? "launch" : "launches"} will be stopped and launched again.
        </p>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="outline" onClick={onSaveOnly}>
            Save only
          </Button>
          <Button onClick={onSaveAndRelaunch}>Save and relaunch</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
