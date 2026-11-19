"use client";

import { createOrbitClient } from "@orbit/api-client";

/**
 * Same-origin by default: the Next.js API proxy route forwards `/api/*` to the control
 * plane using `ORBIT_API_INTERNAL_URL` at runtime. Set NEXT_PUBLIC_ORBIT_API_URL only
 * when the API is served from a different origin (e.g. native apps or split domains).
 */
export const API_BASE_URL =
  process.env.NEXT_PUBLIC_ORBIT_API_URL?.replace(/\/$/, "") ?? "";

const DEV_TOKEN = process.env.NEXT_PUBLIC_ORBIT_DEV_TOKEN ?? null;

export function getAccessToken(): string | null {
  if (typeof window === "undefined") return DEV_TOKEN;
  return window.localStorage.getItem("orbit.access_token") ?? DEV_TOKEN;
}

export function setAccessToken(token: string | null) {
  if (typeof window === "undefined") return;
  if (token && token.trim()) {
    window.localStorage.setItem("orbit.access_token", token.trim());
  } else {
    window.localStorage.removeItem("orbit.access_token");
  }
}

export const api = createOrbitClient({
  baseUrl: API_BASE_URL,
  token: () => getAccessToken(),
});

export function eventStreamUrl(executionId: string): string {
  return `${API_BASE_URL}/api/v1/executions/${executionId}/events/stream`;
}
