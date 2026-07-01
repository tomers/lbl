import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const out = join(root, "..", "assets", "preview");

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
cpSync(join(root, ".output", "public"), out, { recursive: true });

const indexPath = join(out, "index.html");
let html = readFileSync(indexPath, "utf8");
if (!html.includes("<!--LBL_PREVIEW_PAYLOAD-->")) {
  html = html.replace("<script", "<!--LBL_PREVIEW_PAYLOAD-->\n<script");
}
html = normalizeStaticPaths(html);
writeFileSync(indexPath, html);

for (const name of ["200.html", "404.html"]) {
  const path = join(out, name);
  let extra = readFileSync(path, "utf8");
  writeFileSync(path, normalizeStaticPaths(extra));
}

/** Make the prerendered shell work from arbitrary HTTP paths (not just `/`). */
function normalizeStaticPaths(html) {
  return html
    .replaceAll('href="/_nuxt/', 'href="./_nuxt/')
    .replaceAll('src="/_nuxt/', 'src="./_nuxt/')
    .replaceAll('baseURL:"/"', 'baseURL:"./"')
    .replaceAll('buildAssetsDir:"/_nuxt/"', 'buildAssetsDir:"./_nuxt/"');
}
