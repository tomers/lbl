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
| GET | `/api/printers` | Discovered printers (USB bulk, serial, BLE when enabled) |
| GET | `/api/printers/profiles` | Persisted printer profiles (host discovery only) |
| PUT | `/api/printers/profiles` | Upsert a printer profile (host discovery only) |
| DELETE | `/api/printers/profiles/:id` | Remove a printer profile (host discovery only) |
| GET | `/api/printers/profiles/:id/media` | Detected media SKU when connected (host discovery only) |
| GET | `/api/printers/profiles/:id/status` | Print-engine status when connected (host discovery only) |

Host profile routes are mounted only when `LBL_HOST_DISCOVERY` is enabled
(server print mode). With discovery off (browser print mode), those paths
return 404; clients keep printer profiles locally.
| POST | `/api/preview` | Rasterize source via the print pipeline (PNG gallery) |
| POST | `/api/print` | Run the full pipeline and dispatch to a printer |
| POST | `/api/print/file` | Virtual printer: return encoded files inline (raster or vector PDF) |

`POST /api/preview` and `/api/print` accept a source body with one of `text`,
`html`, or `template` (+ `data`, `each`). Printing runs the browser render on a
blocking task and dispatches via the internal spooler.

### Print dispatch modes (`POST /api/print`)

`dispatch_mode` selects where encoded protocol bytes are delivered:

- **`server`** (default): encode and dispatch to a host device via `network`,
  `usb`, `serial`, or `bluetooth`.
- **`client`**: encode only; return protocol bytes for browser-side delivery
  (WebUSB, Web Serial, Web Bluetooth). Optional `printer` catalog key improves
  transport hints.

```json
{
  "text": "Hello",
  "media": "11352",
  "protocol": "dymo-lw",
  "dispatch_mode": "client",
  "printer": "LabelWriter 550"
}
```

Client-mode response:

```json
{
  "dispatch_mode": "client",
  "protocol": "dymo-lw",
  "handshake": "dymo_lw",
  "transport": { "api": "webusb", "filters": [{ "vendorId": 2338, "productId": 40 }] },
  "labels": [{ "index": 0, "filename": "label-0000.bin", "size": 1234, "data_base64": "..." }]
}
```

`handshake` is one of `fire_and_forget`, `dymo_lw`, or `niimbot_poll`.

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

Set `LBL_HOST_DISCOVERY=0` (or `false` / `off`) to disable local device
enumeration on `GET /api/printers`. Host discovery is enabled by default.

CORS is permissive for local development and browser clients.
