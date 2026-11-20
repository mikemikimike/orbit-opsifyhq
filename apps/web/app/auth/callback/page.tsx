"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { completeOidcLogin } from "@/lib/oidc";
import { safeNextPath } from "@/lib/safe-redirect";

export default function OidcCallbackPage() {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void completeOidcLogin()
      .then((next) => router.replace(safeNextPath(next)))
      .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
  }, [router]);

  return (
    <div className="w-full max-w-sm rounded-xl border border-ink-border bg-ink p-6 shadow-overlay text-center sm:p-8">
      {error ? (
        <div role="alert" className="space-y-4">
          <h1 className="font-display text-2xl leading-tight text-ink-foreground">
            Sign-in did not finish
          </h1>
          <p className="break-words text-sm text-ink-muted">{error}</p>
          <Button className="w-full" onClick={() => router.replace("/login")}>
            Back to sign in
          </Button>
        </div>
      ) : (
        <div role="status" className="flex flex-col items-center gap-3 py-2">
          <Loader2 className="h-5 w-5 animate-spin text-ink-accent" aria-hidden />
          <h1 className="font-display text-xl leading-tight text-ink-foreground">Signing you in</h1>
          <p className="text-sm text-ink-muted">This takes a moment.</p>
        </div>
      )}
    </div>
  );
}
