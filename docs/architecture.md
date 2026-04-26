---
id: architecture
title: Architecture
---

# Architecture

The compiler is intentionally phase-oriented.

## Pipeline

1. Lexer tokenizes XLuau and Luau syntax.
2. Parser builds a lightweight recursive statement tree.
3. Phase 5 transforms rewrite classes, interfaces, decorators, and inheritance into Luau-friendly runtime structures while preserving later-phase syntax inside method bodies.
4. Phase 2 lowering rewrites high-impact syntax like ternaries, nullish logic, pipes, destructuring, and switch.
5. Phase 4 transforms expand type-system syntax such as enums, explicit type args, readonly/freeze, and utility types.
6. Resolver handles imports, exports, aliases, barrel files, and target-specific `require` output.
7. Emitter formats resolved modules back into Luau text.

## Key Modules

- `src/lexer.rs`
- `src/parser.rs`
- `src/phase5.rs`
- `src/lowering.rs`
- `src/phase4.rs`
- `src/resolver.rs`
- `src/emitter.rs`
- `src/compiler.rs`

## Documentation Workflow

The root `docs/` directory is the source of truth. The generated GitHub Pages site should only ever consume those files, not duplicate them.
