---
id: migration-guide
title: Migrating from Luau
---

# Migrating from Luau

XLuau is designed so you can adopt it gradually instead of rewriting everything at once.

## Start Small

A good migration path looks like this:

1. keep existing `.luau` and `.lua` files
2. add a few `.xl` files for new code
3. introduce syntax wins first
4. introduce modules, types, and classes later where they help

## Easy Wins First

The simplest early upgrades are usually:

- `const` and `let`
- optional chaining
- nullish coalescing
- destructuring
- `switch`

Example:

```lua
-- Luau
local city = "unknown"
if user and user.profile and user.profile.city ~= nil then
    city = user.profile.city
end

-- XLuau
let city = user?.profile?.city ?? "unknown"
```

## Move Metatable Code Last

If your Luau code already uses hand-written class-like tables, migrate those only when the XLuau class form will actually improve readability.

You do not need to convert every metatable-based object immediately.

## Keep Reading the Output

One of the best ways to learn XLuau is to inspect emitted Luau after each change.

That helps you:

- confirm what a feature really does
- catch surprising output early
- build intuition for which abstractions are worth using

## Suggested Upgrade Order

1. Author new files as `.xl`
2. Replace repetitive nil checks with `?.` and `??`
3. Replace branch-heavy equality chains with `switch`
4. Move utility modules to `import` / `export`
5. Introduce enums and readonly/freeze for shared data models
6. Migrate obvious class-shaped code to `class`

## Keep in Mind

- Later roadmap features are still pending
- The best XLuau code still respects Luau runtime behavior
- Readable emitted output is part of the design, so use it as a learning tool

## Related Reading

- [Quickstart](../quickstart.md)
- [Language Tour](../language-tour.md)
- [Modules](./modules.md)
