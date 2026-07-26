# type-match-macros

Procedural macro implementation for
[`type-match`](https://crates.io/crates/type-match).

This crate provides the `type_match!`, `#[downcastable]`, and
`downcastable_adapter!` expansions. Most users should depend on `type-match`,
which supplies the runtime `Downcast` trait and re-exports these macros.

Documentation and examples are available in the
[main repository](https://github.com/retrokiller543/type-match) and on
[`docs.rs/type-match`](https://docs.rs/type-match).

Licensed under Apache-2.0.
