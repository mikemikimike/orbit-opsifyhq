-- Track consecutive failed executions so a failed run backs off and the runtime
-- continues with a new execution instead of stalling on the same failed one.
ALTER TABLE agent_runtimes
    ADD COLUMN IF NOT EXISTS consecutive_failures integer NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS failed_execution_id uuid NULL;
