# type-match

`type-match` adds match-like concrete-type and structural-pattern matching to
Rust trait objects.

```rust
use type_match::{downcastable, type_match};

#[downcastable]
trait Animal {
    fn speak(&self) -> &'static str;
}

struct Dog {
    loud: bool,
}

impl Animal for Dog {
    fn speak(&self) -> &'static str {
        if self.loud { "WOOF" } else { "woof" }
    }
}

struct Cat;

impl Animal for Cat {
    fn speak(&self) -> &'static str {
        "meow"
    }
}

fn describe(animal: &dyn Animal) -> String {
    type_match! {
        match animal {
            Dog { loud: true } => "a loud dog".into(),
            dog @ Dog { loud: false } => format!("a dog saying {}", dog.speak()),
            Cat => "a cat".into(),
            other => format!("something saying {}", other.speak()),
        }
    }
}

assert_eq!(describe(&Dog { loud: true }), "a loud dog");
assert_eq!(describe(&Cat), "a cat");
```

The macro evaluates its input once, checks arms from top to bottom, and returns
the body of the first matching arm. Types, patterns, bindings, guards, and arm
bodies retain their source spans through procedural-macro expansion.

## Installation

Add the runtime crate to your project:

```toml
[dependencies]
type-match = "0.1"
```

The procedural-macro implementation is re-exported by `type-match`; consumers
do not need to depend on `type-match-macros` directly.

## Making a trait downcastable

Concrete matching requires a way to project a trait object back to
`dyn Any`. There are two supported setup paths.

### Traits you own

Add `#[downcastable]` to the trait:

```rust
use type_match::downcastable;

#[downcastable]
trait Service {
    fn name(&self) -> &'static str;
}
```

The attribute adds `type_match::Downcast` as a supertrait. Every `'static`
sized implementor receives `Downcast` through a blanket implementation, so
individual implementations require no boilerplate:

```rust
# use type_match::downcastable;
# #[downcastable]
# trait Service { fn name(&self) -> &'static str; }
struct Database;

impl Service for Database {
    fn name(&self) -> &'static str {
        "database"
    }
}
```

The equivalent manual declaration is valid, but usually less pleasant:

```rust
trait ManualService: type_match::Downcast {
    fn name(&self) -> &'static str;
}
```

### Traits you cannot modify

If a trait comes from another crate, generate a local facade:

```rust
use type_match::{downcastable_adapter, type_match};

mod dependency {
    pub trait Task {
        fn run(&self) -> &'static str;
    }

    pub struct Build;
    impl Task for Build {
        fn run(&self) -> &'static str { "building" }
    }

    pub struct Test;
    impl Task for Test {
        fn run(&self) -> &'static str { "testing" }
    }
}

downcastable_adapter! {
    pub trait MatchTask: dependency::Task;
}

fn task_name(task: &dyn MatchTask) -> &'static str {
    type_match! {
        match task {
            dependency::Build => "build",
            other => other.run(),
        }
    }
}

assert_eq!(task_name(&dependency::Build), "build");
assert_eq!(task_name(&dependency::Test), "testing");
```

The adapter macro generates a local trait with the requested foreign bounds
plus `Downcast`, followed by a blanket implementation. Concrete values coerce
directly to `&dyn MatchTask`.

Multiple foreign bounds are accepted:

```ignore
downcastable_adapter! {
    pub trait MatchWidget: foreign::Widget + Send + Sync;
}
```

### Why some form of adapter is unavoidable

An already-erased `&dyn ForeignTrait` contains a data pointer and the vtable
for `ForeignTrait`. If that vtable does not expose an `Any` projection, safe
Rust has no general operation that can recover the concrete type.

A default method like this does not solve the problem:

```compile_fail
use std::any::Any;

trait Broken {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

The cast requires `Self: Sized`, but a trait object is intentionally unsized.
Adding `where Self: Sized` would make the method unavailable on `dyn Broken`,
which defeats object-safe downcasting.

Likewise, an inline macro cannot add missing entries to an existing foreign
vtable. It can cast a value while its concrete type is still known, but it
cannot reconstruct that information after the value has become
`&dyn ForeignTrait`.

Consequently:

- use `#[downcastable]` before erasure for a trait you own;
- use `&dyn LocalAdapter` before erasure for a foreign trait;
- if an API gives you only `&dyn ForeignTrait`, matching concrete types is not
  available unless that foreign API already supports downcasting.

This crate deliberately does not use vtable-layout assumptions or unchecked
pointer casts to bypass that boundary.

## Matching syntax

The outer syntax mirrors a Rust `match` expression:

```text
type_match! {
    match VALUE {
        ARMS...
    }
}
```

`VALUE` is an expression producing a shared downcastable trait-object
reference. It is evaluated exactly once.

### Type arms

An unbound type arm checks only the concrete type:

```rust
# use type_match::{downcastable, type_match};
# #[downcastable] trait Value {}
# struct Text; impl Value for Text {}
# struct Number; impl Value for Number {}
fn classify(value: &dyn Value) -> &'static str {
    type_match! {
        match value {
            Text => "text",
            Number => "number",
            _ => "unknown",
        }
    }
}
# assert_eq!(classify(&Text), "text");
```

Qualified and generic types are supported:

```ignore
module::Message => handle_message(),
Wrapper<u8> => handle_byte_wrapper(),
```

### Bound type arms

Use either `@` or `as` to bind the downcast reference:

```rust
# use type_match::{downcastable, type_match};
# #[downcastable] trait Value {}
# struct Text(String); impl Value for Text {}
fn length(value: &dyn Value) -> usize {
    type_match! {
        match value {
            text @ Text => text.0.len(),
            _ => 0,
        }
    }
}
# assert_eq!(length(&Text("hello".into())), 5);
```

The `as` spelling is equivalent:

```ignore
text as Text => text.0.len(),
```

The binding has type `&T`, where `T` is the matched concrete type.

### Struct patterns

Struct patterns perform the downcast and structural match in one generated
`if let` condition:

```rust
# use type_match::{downcastable, type_match};
# #[downcastable] trait Event {}
struct Resize { width: u32, height: u32 }
impl Event for Resize {}

fn orientation(event: &dyn Event) -> &'static str {
    type_match! {
        match event {
            Resize { width, height } if width > height => "landscape",
            Resize { width, height } if height > width => "portrait",
            Resize { .. } => "square",
            _ => "not a resize",
        }
    }
}

assert_eq!(orientation(&Resize { width: 16, height: 9 }), "landscape");
```

Normal Rust field patterns are accepted, including shorthand bindings,
nested patterns, literals, `..`, and `@` subpatterns.

An entire structurally matched value can also be bound:

```ignore
resize @ Resize { width: 0, .. } => reject(resize),
resize as Resize { width, height } => process(resize, width, height),
```

### Tuple-struct patterns

Tuple structs work the same way:

```rust
# use type_match::{downcastable, type_match};
# #[downcastable] trait Command {}
struct Move(i32, i32);
impl Command for Move {}

fn direction(command: &dyn Command) -> &'static str {
    type_match! {
        match command {
            Move(x, _) if *x < 0 => "left",
            Move(x, _) if *x > 0 => "right",
            Move(0, 0) => "still",
            _ => "vertical",
        }
    }
}

assert_eq!(direction(&Move(-1, 0)), "left");
```

For a structural arm, the pattern path is also used as the concrete downcast
type. `Dog { ... }` therefore downcasts to `Dog`, and `Pair(...)` downcasts to
`Pair`.

Enum-variant paths cannot always reveal their owning enum syntactically. For
example, the macro cannot know whether `a::b::Variant` means enum `a::b` or a
struct named `Variant` in module `a::b`. Match the enum type first and use a
normal Rust `match` in the body when that ambiguity matters.

### Guards

Any type, pattern, or wildcard arm may have a guard:

```ignore
dog @ Dog if dog.is_ready() => start(dog),
Dog { age } if *age > 10 => senior(),
_ if use_default => default_value(),
```

A type-arm guard runs only after the concrete type matches. A structural-arm
guard runs only after both the downcast and pattern match. Failed guards fall
through to later arms.

The generated conditions use Rust let-chains, and the arms form one
`if / else if / else` chain rather than independent or nested checks:

```ignore
if let Some(Dog { size: Size::Medium }) = any.downcast_ref::<Dog>()
    && guard
{
    // arm body
} else if any.is::<Cat>() {
    // next arm body
} else {
    // fallback
}
```

Generic arguments remain on the downcast type but are removed from structural
patterns, where Rust can infer them from `Option<&T>`:

```ignore
// Input:
Tagged::<u8> { value: 7 } => lucky(),

// Relevant generated condition:
if let Some(Tagged { value: 7 }) = any.downcast_ref::<Tagged<u8>>() {
    // ...
}
```

### Fallbacks

There are two fallback forms.

An unbound fallback ignores the original value:

```ignore
_ => default_value(),
```

The reserved `other` fallback binds the original trait-object reference, so
trait methods remain available:

```ignore
other => log_and_handle(other),
```

An unguarded `_` or `other` fallback must be the final arm. A guarded `_` may
be followed by more arms.

### Missing fallbacks and unreachable arms

If no fallback arm is present, failure to match any arm invokes:

```rust,ignore
unreachable!("type_match!: no arm matched")
```

Use an explicit unreachable body when a particular concrete type is a known
invariant violation:

```ignore
Impossible => unreachable!("Impossible values are filtered earlier"),
```

`unreachable!` is a panic. It is appropriate only for programmer invariants.
If unmatched values can occur through valid input, return a fallback value,
`Option`, or `Result` instead.

### Arm commas

Expression arms require commas unless they are final:

```ignore
Dog => dog_value(),
Cat => cat_value(),
_ => fallback_value()
```

As with Rust `match`, block arms may omit commas:

```ignore
Dog => {
    prepare();
    dog_value()
}
Cat => {
    cat_value()
}
```

## Evaluation and control-flow semantics

The input expression is assigned to an internal hygienic binding exactly
once. Arms are then evaluated from top to bottom:

1. attempt the concrete downcast;
2. apply the structural pattern, if present;
3. evaluate the guard, if present;
4. evaluate and return the body of the first successful arm.

Later type checks, patterns, guards, and bodies do not run after an arm has
matched. The implementation uses a hygienic labeled block so arm bodies may
produce any common expression type.

The macro does not allocate, clone, or perform string-based type lookup.
Downcasting uses `std::any::Any` and `TypeId` through `Downcast::as_any`.

## Lifetimes and `Any`

`std::any::Any` represents `'static` concrete types. Consequently, matched
implementors cannot contain non-`'static` borrowed data. This is the same
fundamental restriction as direct `Any::downcast_ref`.

The trait-object reference itself may have any borrow lifetime; only the
concrete value's type must satisfy `Any`.

## Tooling and diagnostics

`type_match!` is a procedural macro backed by `syn`. User-written expressions,
types, patterns, bindings, and guards are parsed as native Rust syntax and
interpolated with their original spans. This gives rustc and IDEs substantially
more information than a recursive `macro_rules!` token muncher.

Generated implementation identifiers and labels use mixed-site spans so they
cannot collide with caller names. The runtime crate path is resolved with
`proc-macro-crate`, so the dependency may be renamed in `Cargo.toml`.

Parser diagnostics include:

- an empty `type_match!` body;
- malformed types, patterns, guards, or expressions;
- a non-final unguarded fallback;
- a missing comma after a non-block expression arm.

## Syntax summary

```text
type_match! {
    match value {
        Type => expression,
        binding @ Type => expression,
        binding as Type if guard => expression,

        Struct { fields } => expression,
        binding @ Struct { fields } if guard => expression,
        TupleStruct(patterns...) => expression,

        _ if guard => expression,
        _ => fallback_expression,
        other => bound_fallback_expression,
    }
}
```

Omitting the final fallback is permitted and means that reaching the end is
unreachable.
