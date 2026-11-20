"use client";

import { DocView } from "@/components/docs/doc-view";
import { DOCS_HOME_SLUG } from "@/lib/docs";

export default function DocumentationHomePage() {
  return <DocView slug={DOCS_HOME_SLUG} />;
}
