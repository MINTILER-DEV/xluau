---
id: language-tour
title: Language Tour
---

# Language Tour

This is the fastest overview of the XLuau features you are most likely to use.

## Variables

XLuau supports `let` and `const` in addition to Luau `local`.

```lua
let counter = 0
const APP_NAME = "demo"
local legacy = true
```

In emitted Luau, these become `local`. The difference is at XLuau compile time:

- `const` cannot be reassigned
- `let` is a more familiar mutable local

## Ternaries

```lua
local label = isReady ? "ready" : "waiting"
```

This compiles to Luau `if ... then ... else ...` logic.

## Optional Chaining and Nullish Coalescing

```lua
local city = user?.profile?.city ?? "unknown"
```

Use these when values may be `nil` and you want clean fallback behavior.

## Destructuring

Object destructuring:

```lua
local { name, score: points } = player
```

Array destructuring:

```lua
local [head, _, ...tail] = items
```

You can also use destructuring in function parameters and `for` loops.

## Pipe Operator

```lua
local result = value |> normalize |> encode |> send
```

This is useful when you want left-to-right data flow instead of nested calls.

## Switch

```lua
switch state
case "idle":
    print("waiting")
case "running":
    print("busy")
default:
    print("unknown")
end
```

Use `switch` when several equality-based branches are clearer than chained `if` statements.

## Modules

```lua
import Player, { spawn } from "./player"
export const MAX_HP = 100
export default Player
```

XLuau adds a more structured module layer on top of Luau `require`.

See [Modules](./guides/modules.md) for the full guide.

## Type Features

XLuau currently adds:

- `enum`
- generic constraints with `extends`
- default type parameters on functions
- explicit type arguments at call sites
- readonly/freeze support

Example:

```lua
enum Status { Idle, Running, Done }

function wrap<T extends string>(value: T): T
    return value
end
```

See [Types](./guides/types.md) for details.

## OOP Features

XLuau adds first-class classes:

```lua
class Player {
    name: string

    constructor(name: string)
        self.name = name
    end

    function greet(): string
        return "hi " .. self.name
    end
}
```

It also supports:

- `extends`
- `implements`
- abstract classes and abstract methods
- decorators

See [Classes and Decorators](./guides/classes-and-decorators.md).

## What Is Not Ready Yet

The roadmap still lists later phases that are not implemented yet, such as:

- async/await
- macros
- source maps and richer tooling

Use the [Roadmap Status](./roadmap-status.md) page if you want the implementation view.
