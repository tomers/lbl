<!-- markdownlint-disable-file MD022 MD041 MD036 -->
---json
[
  { "name": "Alice", "title": "Engineer", "url": "https://example.com/alice" },
  { "name": "Bob", "title": "Designer", "url": "https://example.com/bob" }
]
---
**{{ name }}**

*{{ title }}*

{{ "{{qr:" ~ url ~ "}}" }}
