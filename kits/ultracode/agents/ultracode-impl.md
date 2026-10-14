---
name: Ultracode Implementer
description: "Hidden work tier for tickets that change code or files. Same contract as ultracode-worker, with more reasoning. Pin a strong model here (model: <provider::model>)."
icon: code
color: blue
extends: ultracode-worker
hidden: true
reasoning_effort: high
functions:
  - coder::update-file
  - coder::create-file
---
