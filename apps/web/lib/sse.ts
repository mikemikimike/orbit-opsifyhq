import { eventStreamUrl, getAccessToken } from "./api";
import { apiErrorFromResponse, handleUnauthorized } from "./http";

export interface ParsedSseEvent {
  event: string;
  id?: string;
  data: string;
}

/**
 * Read the ORBIT execution SSE stream with `fetch` so the bearer token can be sent in
 * an Authorization header (browser EventSource cannot set headers). Parsing is
 * standards-compliant for `event:`, `id:` and multi-line `data:` fields.
 */
export async function streamExecutionEvents(
  executionId: string,
  options: {
    signal?: AbortSignal;
    lastEventId?: string;
    afterSeq?: number;
    onEvent: (event: ParsedSseEvent) => void;
    /** Called once the server accepted the stream (HTTP 200), before any event. */
    onOpen?: () => void;
    /** Called with an `ApiError` for HTTP failures (check `status`), or the network error. */
    onError?: (error: unknown) => void;
  },
): Promise<void> {
  const params = new URLSearchParams();
  if (options.afterSeq !== undefined) params.set("after_seq", String(options.afterSeq));
  const query = params.toString();
  const url = `${eventStreamUrl(executionId)}${query ? `?${query}` : ""}`;

  try {
    const token = getAccessToken();
    const response = await fetch(url, {
      headers: {
        accept: "text/event-stream",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
        ...(options.lastEventId ? { "last-event-id": options.lastEventId } : {}),
      },
      signal: options.signal,
      cache: "no-store",
    });
    if (!response.ok) {
      if (response.status === 401) handleUnauthorized();
      throw await apiErrorFromResponse(response);
    }
    if (!response.body) throw new Error("The event stream has no body");
    options.onOpen?.();

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });

      let separator = buffer.indexOf("\n\n");
      while (separator !== -1) {
        const frame = buffer.slice(0, separator);
        buffer = buffer.slice(separator + 2);
        const parsed = parseFrame(frame);
        if (parsed) options.onEvent(parsed);
        separator = buffer.indexOf("\n\n");
      }
    }
  } catch (error) {
    if ((error as { name?: string })?.name === "AbortError") return;
    options.onError?.(error);
  }
}

function parseFrame(frame: string): ParsedSseEvent | null {
  const lines = frame.split(/\r?\n/);
  let event = "message";
  let id: string | undefined;
  const data: string[] = [];
  let hasField = false;

  for (const line of lines) {
    if (!line || line.startsWith(":")) continue;
    const index = line.indexOf(":");
    const field = index === -1 ? line : line.slice(0, index);
    let value = index === -1 ? "" : line.slice(index + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    switch (field) {
      case "event":
        event = value || "message";
        hasField = true;
        break;
      case "id":
        id = value;
        hasField = true;
        break;
      case "data":
        data.push(value);
        hasField = true;
        break;
      default:
        break;
    }
  }
  if (!hasField) return null;
  return { event, id, data: data.join("\n") };
}
