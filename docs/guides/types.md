---
id: types-guide
title: Types
---

# Types

XLuau extends Luau’s type ergonomics without hiding the Luau model underneath.

## Enums

```lua
enum Status { Idle, Running, Done }
```

This lowers to:

- a Luau type union
- a frozen runtime table

You can also use explicit values:

```lua
enum HttpMethod { Get = "GET", Post = "POST" }
```

## Generic Constraints

```lua
function echoName<T extends { name: string }>(value: T): T
    return value
end
```

Use `extends` when a generic type must satisfy a structure.

## Default Type Parameters

```lua
function fetchJson<T, Err = string>(url: string): Result<T, Err>
    return nil :: any
end
```

This is helpful when callers usually want one common error type.

## Explicit Type Arguments

```lua
local value = makeDefault<number>()
local result = fetch<User, ApiError>("/api/user")
```

Use this when inference is ambiguous or when you want to make the intended type obvious.

## Readonly

```lua
type Config = {
    readonly host: string,
    timeout: number,
}
```

Readonly fields are enforced by XLuau and emitted according to the configured Luau target.

## Freeze

```lua
const defaults = freeze {
    host = "localhost",
    timeout = 30,
}
```

This becomes `table.freeze(...)` in output and communicates immutability more clearly in source.

## Type Utilities

XLuau also supports built-in utility-style types such as:

- `Partial<T>`
- `Readonly<T>`
- `Pick<T, K>`
- `Omit<T, K>`
- `Record<K, V>`

Example:

```lua
type User = {
    id: string,
    name: string,
    admin: boolean,
}

type PublicUser = Omit<User, "admin">
```

## Best Practices

- Use enums for closed value sets
- Use generic constraints when the body depends on structure
- Reach for explicit type arguments when inference becomes hard to read
- Use `freeze` and `readonly` for shared config and constants

## Related Reading

- [Language Tour](../language-tour.md)
- [Classes and Decorators](./classes-and-decorators.md)
