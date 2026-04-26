---
id: getting-started
title: Getting Started
---

# Getting Started

## Requirements

- Rust toolchain
- Python 3.12+ for the documentation site build

## Compiler Commands

```bash
cargo run -- build
cargo run -- check
```

Run against a specific project or file:

```bash
cargo run -- build path/to/file.xl
cargo run -- check path/to/project
```

## Configuration

Project configuration lives in `xluau.config.json`.

Common fields:

- `include`
- `outDir`
- `baseDir`
- `target`
- `paths`
- `indexFiles`
- `luauTarget`
- `emitReadonly`

## Documentation Site

The documentation site is an MkDocs Material project in `site/` and reads Markdown content directly from `docs/`.

Local docs commands:

```bash
python -m pip install -r site/requirements.txt
mkdocs serve -f site/mkdocs.yml
```
