#![doc = include_str!("../README.md")]

extern crate self as type_match;

#[cfg(test)]
mod test;

use std::any::Any;

pub use type_match_macros::{downcastable, downcastable_adapter, type_match};

/// Object-safe access to a value's [`Any`] representation.
///
/// The blanket implementation covers every `'static` sized type. Trait
/// objects use it through a trait that has `Downcast` as a supertrait; prefer
/// [`downcastable`] or [`downcastable_adapter!`] rather than writing that bound
/// manually.
pub trait Downcast {
    /// Returns this value as an immutable [`Any`] trait object.
    fn as_any(&self) -> &dyn Any;

    /// Returns this value as a mutable [`Any`] trait object.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> Downcast for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
