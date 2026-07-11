# Brother QL setup (Studio + CLI)

Print address and QR labels on Brother QL printers (QL-700 / 800 / 1100
families) from **lbl-print Studio** or the `lbl` CLI. Supported media includes
DK die-cut rolls (e.g. DK-11201 address) and continuous tape
(e.g. DK-22212 / DK-22205).

## What you need

- A catalogued Brother QL model (USB IDs are in the printer catalog)
- DK media that fits the head width (≤ 62 mm for QL-800 family; wider DK for
  QL-1100)
- **Browser mode:** Chrome or Edge with WebUSB (HTTPS or localhost)
- **Server mode:** `lbl-server` on the same machine or LAN as the printer
  (USB or TCP :9100)

## Browser mode (WebUSB)

1. Open Studio → **Printers** → **Connect printer** → **USB**.
2. Select your QL in the browser permission dialog.
3. Studio saves a profile, queries live status (`ESC i S`), and resolves the
   loaded **Paper type** (DK size) when the printer reports it.
4. In the editor, pick media (or accept the detected paper type), design the
   label, then **Print**.

### Tips

- Quit P-touch Editor / Brother iPrint&Label first so WebUSB can claim the
  device.
- On Linux, you may need a udev rule for Brother VID `04f9` (Studio shows a
  hint when relevant).
- Classic Bluetooth SPP on QL-820NWB is not supported yet; use USB or network.

## Server mode (USB or TCP 9100)

### USB

1. Plug in the printer and open Studio against `lbl-server`.
2. **Printers** → **Scan for printers**, then **Add as profile** on the
   discovered QL.

### Network (Wi‑Fi / Ethernet)

Wi‑Fi and Ethernet models (QL-810W, QL-820NWBc, QL-710W, QL-720NW,
QL-1110NWB, …) accept raw raster jobs on **TCP port 9100** once they are on
your LAN.

1. Put the printer on the network with Brother’s Wireless Device Setup Wizard
   (or the printer LCD / WPS).
2. Note the printer’s IP address.
3. In Studio (server mode): **Printers** → **Add network printer**.
4. Choose the QL model, enter `host:9100` (for example `192.168.1.50:9100`),
   and save.
5. Select that profile in Studio and print.

Advanced override: Print modal → **Advanced** → Protocol `brother-ql` and
Target — Network `host:9100`.

## CLI smoke test

```bash
# USB (vid:pid from `lbl device list` or the catalog)
lbl print --protocol brother-ql --usb 04f9:209d --media DK-11201 \
  --markdown $'Home\n\n::: qr\nhttps://example.com\n:::\n'

# Network
lbl print --protocol brother-ql --network 192.168.1.50:9100 --media DK-22212 \
  --markdown 'Continuous tape sample'
```

## Paper type / media detect

When the printer reports media geometry, Studio maps it to a DK SKU and badges
the media picker as **Paper type**. If the selected media differs, Studio warns
before print so you can switch to the loaded roll.

Black/red two-color DK (**DK-22251** / DK-2251) uses Brother’s dual-plane
raster (`w` rows): Studio color directives map red-ish pixels to the low-energy
plane and dark pixels to black. You must select that media SKU when the
black/red roll is loaded (same requirement as Brother’s `--red` flag).

## Related

- [Managing Printers & Media](./printers-media.md)
- Product roadmap Phase 0:
  [`docs/product-roadmap-feature-gaps.md`](../../../docs/product-roadmap-feature-gaps.md)
- Manufacturer matrix:
  [`docs/printer-manufacturer-catalog.md`](../../../docs/printer-manufacturer-catalog.md)
