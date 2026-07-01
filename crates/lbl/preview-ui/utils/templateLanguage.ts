import type { ShjLanguage } from "@speed-highlight/core";

export function languageForTemplateKind(kind: string): ShjLanguage {
  switch (kind) {
    case "html":
    case "template":
      return "html";
    case "markdown":
      return "md";
    default:
      return "plain";
  }
}
