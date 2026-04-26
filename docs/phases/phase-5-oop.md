---
id: phase-5-oop
title: Phase 5 - OOP
---

# Phase 5 - OOP

Status: complete

## Scope

- `class` keyword
- inheritance via `extends`
- interface checking via `implements`
- abstract classes and abstract methods
- built-in decorators
- custom decorator registry support

## Implemented

Phase 5 now runs as a dedicated pre-lowering transform in `src/phase5.rs`, with the compiler pipeline invoking it before Phase 2 so method bodies can still contain later syntax features.

Implemented behavior:

- `class` declarations emit Luau class tables, instance type aliases, class type aliases, constructors, instance methods, and static methods
- `extends` emits parent metatable chaining plus `super(...)` constructor lowering and `super.method(...)` rewrites
- `interface` declarations lower to type aliases and participate in static member-presence checks for `implements`
- `abstract class` and `abstract function` participate in compile-time enforcement
- built-in decorators:
  - `@singleton`
  - `@memoize`
  - `@deprecated("message")`
  - `@readonly`
  - `@sealed`
  - `@abstract`
- custom decorators are emitted through the configured `decoratorLibrary`

## Notes

- Custom decorators require `decoratorLibrary` in `xluau.config.json`.
- Readonly properties are enforced during XLuau compilation, while emitted Luau also carries a readonly marker table on the class.
- Compiler coverage includes parser regressions, Phase 5 transform tests, and an end-to-end build test for combined OOP features.

## Key Files

- `src/phase5.rs`
- `src/compiler.rs`
- `src/parser.rs`
- `src/lowering.rs`
- `src/lexer.rs`
