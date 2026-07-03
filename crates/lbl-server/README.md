# lbl-server

The HTTP API (axum) for programmatic access to the `lbl` pipeline.

## Endpoints

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/api/health` | Liveness check |
| GET | `/api/config` | Effective merged configuration |
| GET | `/api/config/sources` | Provenance (which layer set each value) |
| GET | `/api/catalog` | List media entries |
| GET | `/api/catalog/:key` | Resolve a media SKU/alias |
| GET | `/api/catalog/compatible?printer=` | Media compatible with a printer |
| GET | `/api/printers` | Discovered (USB) printers |
| GET | `/api/printers/profiles` | Persisted printer profiles |
| PUT | `/api/printers/profiles` | Upsert a printer profile |
| DELETE | `/api/printers/profiles/:id` | Remove a printer profile |
| POST | `/api/preview` | Transpile source into preview HTML (gallery) |
| POST | `/api/print` | Run the full pipeline and dispatch to a printer |
| POST | `/api/print/file` | Virtual printer: return encoded files inline (raster or vector PDF) |

`POST /api/preview` and `/api/print` accept a source body with one of `text`,
`html`, or `template` (+ `data`, `each`). Printing runs the browser render on a
blocking task and dispatches via the internal spooler.

### Virtual file export (`POST /api/print/file`)

Set `"protocol": "virtual"`. Optional fields:

```json
{
  "text": "Hello {{qr:https://x}}",
  "media": "30252",
  "protocol": "virtual",
  "export_mode": "vector",
  "media_type": "png"
}
```

- **`export_mode`**: `"raster"` (default) or `"vector"` / `"pdf"`.
- **`media_type`**: `"png"`, `"bmp"`, `"tiff"`, `"gif"`, or `"pbm"` — raster
  only; ignored when `export_mode` is `vector` (response is always PDF).

`POST /api/preview` and hardware `/api/print` do not use `export_mode`.

## Run

```bash
lbl-server --bind 127.0.0.1:8787
```

CORS is permissive for local development and browser clients.
