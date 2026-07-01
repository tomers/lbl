export interface PreviewPrinter {
  key?: string | null;
  name?: string | null;
  brand?: string | null;
  protocol: string;
  dpi: number;
  max_width_mm?: number | null;
  transport?: string | null;
}

export interface PreviewMedia {
  sku?: string | null;
  name?: string | null;
  width_mm: number;
  length_mm?: number | null;
  continuous: boolean;
  dpi: number;
  material?: string | null;
  color?: string | null;
}

export interface PreviewTemplate {
  kind: string;
  path?: string | null;
  each?: string | null;
  body: string;
}

export interface PreviewLabel {
  index: number;
  image: string;
  width: number;
  height: number;
  record: unknown;
}

export interface PreviewPayload {
  count: number;
  printer: PreviewPrinter;
  media: PreviewMedia;
  template: PreviewTemplate;
  data: unknown;
  labels: PreviewLabel[];
}

declare global {
  interface Window {
    __LBL_PREVIEW__?: PreviewPayload;
  }
}

export {};
