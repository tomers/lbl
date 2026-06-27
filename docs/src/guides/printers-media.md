# Managing Printers & Media

## Discover printers

```bash
lbl device list           # via the orchestrator
lbl-device list           # standalone
```

USB printers are matched against a known-printer table to suggest a model and
protocol. In the web app, the **Printers** page lists discovered devices and
lets you adopt one as a saved profile.

## Profiles

Profiles persist a printer's desired configuration (name, model, protocol,
capabilities, transport, default media) so it survives disconnects. They're
stored in `printers.toml` next to your config.

Manage them via the API (`/api/printers/profiles`).

## Media

Refer to media by SKU:

```bash
lbl-catalog compatible --printer "LabelWriter 550"
lbl print --media 11352 ...        # resolves to 25x54mm return-address labels
```

Or specify dimensions directly:

```bash
lbl print --width-mm 56 --length-mm 89 --dpi 300 ...   # fixed
lbl print --width-mm 56 --dpi 203 ...                  # continuous
```

The **Media** page in the web app browses the catalog with images, lets you
filter by printer compatibility, and links out to purchase pages.
