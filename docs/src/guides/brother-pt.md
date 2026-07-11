# Brother P-touch / TZe setup

Print laminated TZe tape labels on Brother P-touch printers (PT-P700 / H500 /
E500 / D600 family) from **lbl-print Studio** or the `lbl` CLI. Protocol id:
`brother-pt`.

## What you need

- A catalogued PT model (USB IDs in the printer catalog)
- TZe laminated tape ≤ 24 mm (e.g. TZe-231 12 mm)
- **Browser mode:** Chrome or Edge with WebUSB
- **Server mode:** `lbl-server` with USB (or TCP :9100 on Wi‑Fi models such as
  PT-P750W)

## Browser (WebUSB)

1. Studio → **Printers** → **Connect printer** → **USB**.
2. Select the P-touch in the browser dialog.
3. Pick a TZe media SKU, design the label, then **Print**.

Quit P-touch Editor first so WebUSB can claim the device.

## CLI smoke test

```bash
lbl print --protocol brother-pt --usb 04f9:2061 --media TZe-231 \
  --markdown 'Cable A1'

lbl print --protocol brother-pt --network 192.168.1.50:9100 --media TZe-251 \
  --markdown 'Shelf B'
```

## Live status / tape detect

USB models answer `ESC i S` with a 32-byte status block (same request as QL,
different media-type codes). Studio and `lbl-server` use it for ready/error
state and to map reported tape width to a TZe catalog SKU (color/finish is not
reported uniquely — black-on-white is preferred when several SKUs share a
width).

## Notes

- Head geometry is 128 dots @ 180 dpi (PT-P700 reference). Wider PT-P900
  (36 mm / 360 dpi) is not covered yet.
- Classic Bluetooth models (e.g. PT-P710BT / Cube) need a separate BLE path.

## Related

- [Brother QL setup](./brother-ql.md) (DK rolls — different protocol)
- [Managing Printers & Media](./printers-media.md)
