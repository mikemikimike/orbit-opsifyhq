"use client";

import { useParams } from "next/navigation";

import { DocView } from "@/components/docs/doc-view";

export default function DocumentationPage() {
  const params = useParams<{ slug: string[] }>();
  const parts = Array.isArray(params.slug) ? params.slug : [params.slug];
  return <DocView slug={parts.map(decodeURIComponent).join("/")} />;
}
