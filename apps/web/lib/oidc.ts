"use client";

import { safeNextPath } from "@/lib/safe-redirect";

const VERIFIER_KEY = "orbit.oidc.verifier";
const STATE_KEY = "orbit.oidc.state";
const NEXT_KEY = "orbit.oidc.next";

type OidcConfig = {
  issuer: string;
  clientId: string;
  redirectUri: string;
  scopes: string[];
  authorizationEndpoint: string;
  endSessionEndpoint?: string | null;
};

function base64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function randomString(bytes = 32): string {
  const value = new Uint8Array(bytes);
  crypto.getRandomValues(value);
  return base64Url(value);
}

async function challenge(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(verifier),
  );
  return base64Url(new Uint8Array(digest));
}

async function fetchConfig(): Promise<OidcConfig> {
  const response = await fetch("/api/v1/auth/oidc/config", { cache: "no-store" });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as
      | { error?: { message?: string } }
      | null;
    throw new Error(body?.error?.message ?? `OIDC config failed: HTTP ${response.status}`);
  }
  return (await response.json()) as OidcConfig;
}

export async function beginOidcLogin(next = "/") {
  const config = await fetchConfig();
  const verifier = randomString(48);
  const state = randomString(24);
  const codeChallenge = await challenge(verifier);
  sessionStorage.setItem(VERIFIER_KEY, verifier);
  sessionStorage.setItem(STATE_KEY, state);
  sessionStorage.setItem(NEXT_KEY, safeNextPath(next));
  const url = new URL(config.authorizationEndpoint);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("client_id", config.clientId);
  url.searchParams.set("redirect_uri", config.redirectUri);
  url.searchParams.set("scope", config.scopes.join(" "));
  url.searchParams.set("state", state);
  url.searchParams.set("code_challenge", codeChallenge);
  url.searchParams.set("code_challenge_method", "S256");
  window.location.assign(url.toString());
}

export async function completeOidcLogin(): Promise<string> {
  const params = new URLSearchParams(window.location.search);
  const error = params.get("error");
  if (error) {
    throw new Error(params.get("error_description") ?? error);
  }
  const code = params.get("code");
  const state = params.get("state");
  const expectedState = sessionStorage.getItem(STATE_KEY);
  const verifier = sessionStorage.getItem(VERIFIER_KEY);
  const next = safeNextPath(sessionStorage.getItem(NEXT_KEY));
  if (!code || !state || !expectedState || state !== expectedState || !verifier) {
    throw new Error("OIDC state or code is invalid; start the login again.");
  }
  const response = await fetch("/api/v1/auth/oidc/token", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ code, codeVerifier: verifier }),
  });
  const body = (await response.json().catch(() => null)) as
    | {
        accessToken?: string;
        idToken?: string;
        error?: { message?: string };
      }
    | null;
  if (!response.ok) {
    throw new Error(body?.error?.message ?? `OIDC token exchange failed: HTTP ${response.status}`);
  }
  const token = body?.idToken ?? body?.accessToken;
  if (!token) throw new Error("OIDC provider returned no token");
  sessionStorage.removeItem(VERIFIER_KEY);
  sessionStorage.removeItem(STATE_KEY);
  sessionStorage.removeItem(NEXT_KEY);
  localStorage.setItem("orbit.access_token", token);
  return next;
}
