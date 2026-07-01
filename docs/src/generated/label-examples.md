# Fixed-size label examples

Commands show content and layout flags only. Media size, DPI, protocol, and output path come from project config (`lbl.toml`) or environment.

The cases below mirror the guides: getting started, printing text, batch printing, configuration, rendering quality, and printers & media. Regenerate previews with `just doc-examples`.

## DYMO 11352 · 25×54 mm

```bash
lbl print --text 'Hello {{qr:https://example.com}}'
```

<img src="images/hello-qr.png" alt="DYMO 11352 · 25×54 mm" width="320"/>

## DYMO 11352 · 25×54 mm

```bash
lbl print --text Hello
```

<img src="images/hello.png" alt="DYMO 11352 · 25×54 mm" width="320"/>

## DYMO 99014 · 54×101 mm

```bash
lbl print --template card.html --template-format html --data people.json --one
```

<img src="images/batch-card.png" alt="DYMO 99014 · 54×101 mm" width="320"/>

## NIIMBOT 12×30 mm @ 203 dpi

```bash
lbl print --template 'User #{{ it }}' --data 1 --padding-mm 0
```

<img src="images/user-number.png" alt="NIIMBOT 12×30 mm @ 203 dpi" width="320"/>

## NIIMBOT 12×30 mm @ 203 dpi

```bash
lbl print --text Hi --padding-mm 0
```

<img src="images/hi-no-padding.png" alt="NIIMBOT 12×30 mm @ 203 dpi" width="320"/>

## NIIMBOT 12×22 mm @ 203 dpi

```bash
lbl print --text Hello
```

<img src="images/hello-niimbot.png" alt="NIIMBOT 12×22 mm @ 203 dpi" width="320"/>

## DYMO 11352 · 25×54 mm · supersample 4

```bash
lbl print --text Hello --supersample 4
```

<img src="images/hello-supersample.png" alt="DYMO 11352 · 25×54 mm · supersample 4" width="320"/>

## NIIMBOT 12×40 mm @ 203 dpi

```bash
lbl print --text Ship --padding-mm 0
```

<img src="images/niimbot-tape.png" alt="NIIMBOT 12×40 mm @ 203 dpi" width="320"/>

## 56×89 mm @ 300 dpi

```bash
lbl print --text 'Receipt line'
```

<img src="images/fixed-dimensions.png" alt="56×89 mm @ 300 dpi" width="320"/>

