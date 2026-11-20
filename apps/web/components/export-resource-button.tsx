"use client";

import { Download } from "lucide-react";
import { toast } from "sonner";

import { Button, type ButtonProps } from "@/components/ui/button";
import { downloadExport, type ExportFormat } from "@/lib/portable";
import { errorMessage } from "@orbit/api-client";

/**
 * Download a secret-free, portable export of a shareable resource (YAML by default;
 * `markdown` gives a SKILL.md for skills). Credentials are never included: the file
 * says so, and they are set again after importing.
 */
export async function exportResource(
  resource: string,
  resourceId: string,
  name: string,
  format: ExportFormat = "yaml",
) {
  try {
    await downloadExport(resource, resourceId, name, format);
    toast.success(format === "markdown" ? "SKILL.md downloaded" : "Export downloaded");
  } catch (error) {
    toast.error(`Export failed: ${errorMessage(error)}`);
  }
}

/**
 * Export as a button. Inside an overflow menu, call `exportResource` from a
 * `DropdownMenuItem` instead.
 */
export function ExportResourceButton({
  resource,
  resourceId,
  name,
  format,
  size = "sm",
  variant = "ghost",
}: {
  resource: string;
  resourceId: string;
  name: string;
  format?: ExportFormat;
  size?: ButtonProps["size"];
  variant?: ButtonProps["variant"];
}) {
  return (
    <Button
      type="button"
      size={size}
      variant={variant}
      onClick={() => void exportResource(resource, resourceId, name, format)}
    >
      <Download /> Export
    </Button>
  );
}
