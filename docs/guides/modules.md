---
id: modules-guide
title: Modules
---

# Modules

XLuau gives Luau projects a friendlier module syntax while still compiling down to `require`.

## Why Use XLuau Modules

Plain Luau modules are flexible, but they leave a lot of structure up to convention. XLuau adds:

- named imports
- default exports
- namespace imports
- re-exports
- type-only imports and exports
- path alias support

## Imports

Named import:

```lua
import { add, subtract } from "./math"
```

Aliased import:

```lua
import { add as sum } from "./math"
```

Default import:

```lua
import Player from "./Player"
```

Namespace import:

```lua
import * as utils from "./utils"
```

Side-effect import:

```lua
import "./bootstrap"
```

Type-only import:

```lua
import type { PlayerState } from "./types"
```

## Exports

Named export:

```lua
export const MAX_SCORE = 100
```

Export an existing value:

```lua
local version = "1.0.0"
export { version }
```

Default export:

```lua
export default Player
```

Re-export:

```lua
export { add } from "./math"
export * from "./helpers"
```

## Barrel Files

XLuau can resolve directory imports through configured index files such as `init.xl`.

Example:

```lua
-- src/utils/init.xl
export { clamp } from "./numbers"
export { trim } from "./strings"
```

Then:

```lua
import { clamp } from "./utils"
```

## Path Aliases

In `xluau.config.json`:

```json
{
  "paths": {
    "@utils": "./src/utils",
    "@models": "./src/models"
  }
}
```

Then:

```lua
import { clamp } from "@utils"
import User from "@models/User"
```

## Runtime Shape

XLuau implements exports through an `_exports` table convention in emitted Luau.

That means default exports are stored under `__default` in the generated module output.

You do not usually need to think about that while authoring XLuau, but it helps when reading emitted code.

## Best Practices

- Prefer named exports for utility-heavy modules
- Use default exports for a module’s main thing
- Keep barrel files small and intentional
- Use type-only imports when you are only sharing types

## Related Reading

- [Language Tour](../language-tour.md)
- [Migrating from Luau](./migrating-from-luau.md)
