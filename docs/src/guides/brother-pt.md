# Brother P-touch / TZe setup

Print laminated TZe tape labels on Brother P-touch printers from **lbl-print
Studio** or the `lbl` CLI. Protocol id: `brother-pt`.

## Families

| Family | Head | DPI | Max tape | Typical models |
| --- | ---: | ---: | ---: | --- |
| P700-class | 128-dot | 180 | 24 mm | PT-P700, P750W, H500, E500, D600, **P710BT Cube Plus** |
| P900-class | 560-dot | 360 | 36 mm | PT-P900, P900W, P950NW, P910BT |

Head geometry is chosen from the printer catalog `max_width_mm` (> 24 mm →
P900 layout).

## What you need

- A catalogued PT model
- Matching TZe tape (e.g. TZe-231 12 mm, or TZe-261 36 mm on P900)
- **Browser mode:** Chrome or Edge with WebUSB (USB models) or Web Serial
  (Bluetooth Classic Cube after OS pairing)
- **Server mode:** `lbl-server` with USB, TCP `:9100` (Wi‑Fi models), or serial

## Browser (WebUSB)

1. Studio → **Printers** → **Connect printer** → **USB**.
2. Select the P-touch in the browser dialog.
3. Studio can query live status (`ESC i S`) and map tape width → TZe SKU.
4. Confirm media, design the label, then **Print**.

Quit P-touch Editor first so WebUSB can claim the device.

## Cube / Bluetooth Classic (P710BT, P300BT)

These models use **Bluetooth Classic SPP**, not BLE. Web Bluetooth cannot talk
to them. Options:

1. **USB** (P710BT): use WebUSB as above (`04f9:20af`).
2. **OS-paired serial:** pair in system Bluetooth settings, then open the
   resulting serial port in Studio (Web Serial) or on the server:

```bash
# Linux example after pairing
sudo rfcomm bind 0 <bt-address>
lbl print --protocol brother-pt --serial /dev/rfcomm0:9600 --media TZe-231 \
  --markdown 'Cable A1'
```

## CLI smoke test

```bash
# P700-class USB
lbl print --protocol brother-pt --usb 04f9:2061 --media TZe-231 \
  --markdown 'Cable A1'

# P900-class USB (36 mm)
lbl print --protocol brother-pt --usb 04f9:2083 --media TZe-261 \
  --markdown 'Shelf B'

# Wi‑Fi models (TCP :9100)
lbl print --protocol brother-pt --network 192.168.1.50:9100 --media TZe-251 \
  --markdown 'Shelf B'
```

## Live status / tape detect

USB models answer `ESC i S` with a 32-byte status block. Studio and
`lbl-server` use it for ready/error state and to map reported tape width to a
TZe catalog SKU (color/finish is not unique — black-on-white is preferred when
several SKUs share a width).

## Related

- [Brother PT raster protocol](../reference/brother-pt-raster.md) — opcode
  (`G`/`0x47`), PackBits under `M 02`, multi-page `0x0C`/`0x1A`, RE sources
- [Brother QL setup](./brother-ql.md) (DK rolls — different protocol)
- [Managing Printers & Media](./printers-media.md)
