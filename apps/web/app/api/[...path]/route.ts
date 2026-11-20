import { type NextRequest } from "next/server";

/**
 * Same-origin API proxy used by the web container and local development.
 *
 * The browser always talks to `/api/v1/...` on its own origin. This handler forwards
 * requests to the control plane using the server-side `ORBIT_API_INTERNAL_URL`
 * (read at runtime, not build time) and streams responses back, including SSE.
 *
 * In Kubernetes the ingress can route `/api/*` directly to the API service; this
 * handler is then bypassed. In Docker Compose it removes any CORS or mixed-origin
 * concerns.
 */
export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const HOP_BY_HOP = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
  "content-length",
  "content-encoding",
]);

function upstreamBase(): string {
  return (
    process.env.ORBIT_API_INTERNAL_URL?.replace(/\/$/, "") ??
    "http://localhost:8080"
  );
}

async function proxy(request: NextRequest, path: string[]): Promise<Response> {
  const incoming = new URL(request.url);
  const target = `${upstreamBase()}/api/${path.join("/")}${incoming.search}`;

  const headers = new Headers();
  for (const [key, value] of request.headers.entries()) {
    if (HOP_BY_HOP.has(key.toLowerCase()) || key.toLowerCase() === "host") continue;
    headers.set(key, value);
  }
  // Development convenience: when the control plane runs in dev auth mode, the web
  // container may inject the dev token server-side so the browser never handles it.
  // This is gated by an explicit env var and must not be set for OIDC deployments.
  const devToken = process.env.ORBIT_WEB_DEV_TOKEN;
  if (devToken && !headers.has("authorization")) {
    headers.set("authorization", `Bearer ${devToken}`);
  }

  headers.set("x-forwarded-host", incoming.host);
  headers.set("x-forwarded-proto", incoming.protocol.replace(":", ""));

  const init: RequestInit & { duplex?: "half" } = {
    method: request.method,
    headers,
    redirect: "manual",
    cache: "no-store",
  };

  if (!["GET", "HEAD", "OPTIONS"].includes(request.method)) {
    init.body = request.body;
    init.duplex = "half";
  }

  const upstream = await fetch(target, init);

  const responseHeaders = new Headers();
  for (const [key, value] of upstream.headers.entries()) {
    if (HOP_BY_HOP.has(key.toLowerCase())) continue;
    responseHeaders.set(key, value);
  }
  responseHeaders.set("cache-control", "no-store");

  return new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: responseHeaders,
  });
}

type RouteContext = { params: Promise<{ path: string[] }> };

export async function GET(request: NextRequest, context: RouteContext) {
  const { path } = await context.params;
  return proxy(request, path);
}
export async function POST(request: NextRequest, context: RouteContext) {
  const { path } = await context.params;
  return proxy(request, path);
}
export async function PATCH(request: NextRequest, context: RouteContext) {
  const { path } = await context.params;
  return proxy(request, path);
}
export async function PUT(request: NextRequest, context: RouteContext) {
  const { path } = await context.params;
  return proxy(request, path);
}
export async function DELETE(request: NextRequest, context: RouteContext) {
  const { path } = await context.params;
  return proxy(request, path);
}
