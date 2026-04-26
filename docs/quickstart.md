---
id: quickstart
title: Quickstart
---

# Quickstart

This page gets you from zero to a working XLuau project as quickly as possible.

## 1. Create a Project

Use a simple structure like this:

```text
my-project/
  xluau.config.json
  src/
    main.xl
```

Minimal `xluau.config.json`:

```json
{
  "version": 1,
  "include": ["src/**/*.xl"],
  "outDir": "out",
  "baseDir": "src",
  "target": "filesystem"
}
```

## 2. Write Your First File

Create `src/main.xl`:

```lua
const greeting = "hello"
let target = "world"

local message = greeting .. ", " .. target
print(message)
```

XLuau accepts plain Luau syntax too, so you can adopt it gradually.

## 3. Build and Check

From the repository root:

```bash
cargo run -- build
cargo run -- check
```

If your project is somewhere else:

```bash
cargo run -- build path/to/file.xl
cargo run -- check path/to/project
```

## 4. Understand the Output

XLuau writes compiled Luau into your configured output directory.

For the example above, you should get something like:

```lua
local greeting = "hello"
local target = "world"
local message = greeting .. ", " .. target
print(message)
```

That is the core XLuau promise: nicer authoring syntax, readable Luau output.

## 5. Try a Few Real Features

### Safer nil access

```lua
local city = user?.profile?.city ?? "unknown"
```

### Imports and exports

```lua
import { sum } from "./math"
print(sum(2, 3))
```

### Classes

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

## Next Steps

- Read the [Language Tour](./language-tour.md) next.
- Use [Getting Started](./getting-started.md) for project and docs-site details.
- Jump to [Modules](./guides/modules.md) or [Classes and Decorators](./guides/classes-and-decorators.md) if those are your immediate focus.
