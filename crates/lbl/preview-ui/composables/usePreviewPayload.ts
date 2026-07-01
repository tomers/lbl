import type { PreviewLabel, PreviewPayload } from "~/types/preview";

export function usePreviewPayload() {
  const payload = useState<PreviewPayload | null>("preview-payload", () => {
    if (import.meta.client && window.__LBL_PREVIEW__) {
      return window.__LBL_PREVIEW__;
    }
    return null;
  });

  return { payload };
}

export function flattenValues(value: unknown): string[] {
  if (value == null) {
    return [];
  }
  if (typeof value === "string") {
    return [value];
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return [String(value)];
  }
  if (Array.isArray(value)) {
    return value.flatMap(flattenValues);
  }
  if (typeof value === "object") {
    return Object.values(value as Record<string, unknown>).flatMap(flattenValues);
  }
  return [];
}

export function recordMatchesQuery(record: unknown, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) {
    return true;
  }
  return flattenValues(record).some((value) => value.toLowerCase().includes(q));
}

export function filterLabels(labels: PreviewLabel[], query: string): PreviewLabel[] {
  return labels.filter((label) => recordMatchesQuery(label.record, query));
}
