---
id: classes-guide
title: Classes and Decorators
---

# Classes and Decorators

Phase 5 adds a real OOP authoring layer on top of Luau’s metatable model.

## Basic Class

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

Use classes when you want a clearer source representation than hand-written metatable code.

## Static Methods

```lua
class Player {
    static function create(name: string): Player
        return Player.new(name)
    end
}
```

## Inheritance

```lua
class Animal {
    constructor(name: string)
        self.name = name
    end
}

class Dog extends Animal {
    constructor(name: string, breed: string)
        super(name)
        self.breed = breed
    end
}
```

You can also call parent methods:

```lua
function speak(): string
    return super.speak() .. "!"
end
```

## Interfaces

```lua
interface Serializable {
    serialize: (self: Serializable) -> string
}

class SaveData implements Serializable {
    function serialize(): string
        return "{}"
    end
}
```

XLuau performs static member-presence checks for `implements`.

## Abstract Classes

```lua
abstract class Shape {
    abstract function area(): number
}
```

Rules:

- abstract classes cannot be instantiated directly
- non-abstract subclasses must implement inherited abstract methods

## Decorators

XLuau currently supports built-in decorators and optional custom decorators.

### `@singleton`

```lua
@singleton
class ConfigService {
}
```

Subsequent `new()` calls return the same cached instance.

### `@memoize`

```lua
class Hasher {
    @memoize
    function compute(data: string): string
        return expensiveHash(data)
    end
}
```

### `@deprecated`

```lua
class Api {
    @deprecated("use requestV2")
    function requestV1(): nil
        return nil
    end
}
```

### `@readonly`

```lua
class User {
    @readonly
    id: string
}
```

Assignments outside the constructor are rejected by XLuau.

### `@sealed`

Use `@sealed` when a class should not be subclassed.

### `@abstract`

You can also express abstract intent through decorators when appropriate.

## Custom Decorators

Custom decorators require `decoratorLibrary` in `xluau.config.json`.

Example:

```json
{
  "decoratorLibrary": "./decorators.lua"
}
```

Then you can author:

```lua
class Cache {
    @trace
    static function warm(): nil
        return nil
    end
}
```

## Best Practices

- Use classes for stateful domain objects, not every table
- Prefer interfaces for clear contracts between modules
- Use abstract classes when shared behavior matters
- Keep decorators purposeful; they are best for cross-cutting behavior

## Related Reading

- [Language Tour](../language-tour.md)
- [Types](./types.md)
