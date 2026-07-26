#![cfg(test)]

use std::cell::Cell;

use crate::{downcastable, downcastable_adapter, type_match};

#[downcastable]
trait Animal {
    fn speak(&self) -> String;
}

#[downcastable]
trait Bird: Animal {
    fn fly(&self) -> &'static str;
}

enum Size {
    Small,
    Medium,
    Large,
}

struct Dog {
    size: Size,
}

impl Animal for Dog {
    fn speak(&self) -> String {
        match self.size {
            Size::Small => "yip".to_string(),
            Size::Medium => "woof".to_string(),
            Size::Large => "bark".to_string(),
        }
    }
}

struct Cat;

impl Animal for Cat {
    fn speak(&self) -> String {
        "meow".to_string()
    }
}

struct Fox;

impl Animal for Fox {
    fn speak(&self) -> String {
        "ring-ding-ding".to_string()
    }
}

struct Cow;

impl Animal for Cow {
    fn speak(&self) -> String {
        "moo".to_string()
    }
}

struct Parrot(Size);

impl Animal for Parrot {
    fn speak(&self) -> String {
        "squawk".to_string()
    }
}

impl Bird for Parrot {
    fn fly(&self) -> &'static str {
        "flap flap"
    }
}

struct Tagged<T: 'static> {
    value: T,
}

impl Animal for Tagged<u8> {
    fn speak(&self) -> String {
        self.value.to_string()
    }
}

mod nested {
    pub(super) struct Otter;

    impl super::Animal for Otter {
        fn speak(&self) -> String {
            "chirp".to_string()
        }
    }
}

mod foreign {
    pub trait Instrument {
        fn sound(&self) -> &'static str;
    }

    pub struct Bell;
    pub struct Drum;

    impl Instrument for Bell {
        fn sound(&self) -> &'static str {
            "ding"
        }
    }

    impl Instrument for Drum {
        fn sound(&self) -> &'static str {
            "boom"
        }
    }
}

downcastable_adapter! {
    trait MatchInstrument: foreign::Instrument;
}

fn instrument_name(instrument: &dyn MatchInstrument) -> &'static str {
    type_match! {
        match instrument {
            foreign::Bell => "bell",
            other => other.sound()
        }
    }
}

fn speak(animal: &dyn Animal) -> String {
    type_match! {
        match animal {
            dog @ Dog => { format!("The dog says {}", dog.speak()) },
            cat as Cat => { format!("The cat says {}", cat.speak()) },
            fox @ Fox => { format!("The fox says {}", fox.speak()) },
            other => format!("Other says {}", other.speak())
        }
    }
}

fn guarded_speak(animal: &dyn Animal) -> &'static str {
    type_match! {
        match animal {
            dog as Dog if dog.speak() == "woof" => "medium dog",
            Dog => "another dog",
            _ => "not a dog"
        }
    }
}

fn pattern_speak(animal: &dyn Animal) -> &'static str {
    type_match! {
        match animal {
            Dog { size: Size::Small } => "small dog",
            dog as Dog { size: Size::Medium } if dog.speak() == "woof" => "medium dog",
            dog @ Dog { size: Size::Large } => {
                let _ = dog;
                "large dog"
            }
            Parrot(Size::Small) => "small parrot",
            Parrot(size @ Size::Large) => {
                let _ = size;
                "large parrot"
            }
            Tagged::<u8> { value: 7 } => "lucky tag",
            Tagged<u8> => "another tag",
            nested::Otter => "otter",
            _ => "other"
        }
    }
}

fn comma_less_blocks(animal: &dyn Animal) -> &'static str {
    type_match! {
        match animal {
            Dog => { "dog" }
            Cat => { "cat" }
            other => {
                let _ = other;
                "other"
            }
        }
    }
}

fn guarded_fallback(animal: &dyn Animal, calls: &Cell<u8>) -> &'static str {
    type_match! {
        match animal {
            Dog if {
                calls.set(calls.get() + 1);
                false
            } => "guarded dog",
            _ if animal.speak() == "meow" => "guarded fallback",
            _ => "final fallback",
        }
    }
}

fn observed<'a>(animal: &'a dyn Animal, calls: &Cell<u8>) -> &'a dyn Animal {
    calls.set(calls.get() + 1);
    animal
}

fn count_evaluations<'a>(animal: &'a dyn Animal, calls: &Cell<u8>) -> &'a dyn Animal {
    type_match! {
        match observed(animal, calls) {
            other => other
        }
    }
}

fn explicitly_unreachable(animal: &dyn Animal) -> String {
    type_match! {
        match animal {
            Dog => "dog".to_string(),
            Fox => unreachable!("Fox arm")
        }
    }
}

fn implicitly_unreachable(animal: &dyn Animal) -> String {
    type_match! {
        match animal {
            Dog => "dog".to_string()
        }
    }
}

#[test]
fn test_matching() {
    let small_dog = Dog { size: Size::Small };
    let medium_dog = Dog { size: Size::Medium };
    let large_dog = Dog { size: Size::Large };
    let cat = Cat;
    let fox = Fox;
    let cow = Cow;

    assert_eq!(speak(&small_dog), "The dog says yip");
    assert_eq!(speak(&medium_dog), "The dog says woof");
    assert_eq!(speak(&large_dog), "The dog says bark");
    assert_eq!(speak(&cat), "The cat says meow");
    assert_eq!(speak(&fox), "The fox says ring-ding-ding");
    assert_eq!(speak(&cow), "Other says moo");
    assert_eq!(guarded_speak(&small_dog), "another dog");
    assert_eq!(guarded_speak(&medium_dog), "medium dog");
    assert_eq!(guarded_speak(&cat), "not a dog");
}

#[test]
#[should_panic(expected = "Fox arm")]
fn explicit_unreachable_arm_panics() {
    explicitly_unreachable(&Fox);
}

#[test]
#[should_panic(expected = "no arm matched")]
fn missing_final_arm_is_implicitly_unreachable() {
    implicitly_unreachable(&Cat);
}

#[test]
fn parses_struct_and_tuple_struct_patterns() {
    assert_eq!(pattern_speak(&Dog { size: Size::Small }), "small dog");
    assert_eq!(pattern_speak(&Dog { size: Size::Medium }), "medium dog");
    assert_eq!(pattern_speak(&Dog { size: Size::Large }), "large dog");
    assert_eq!(pattern_speak(&Parrot(Size::Small)), "small parrot");
    assert_eq!(pattern_speak(&Parrot(Size::Large)), "large parrot");
    assert_eq!(pattern_speak(&Parrot(Size::Medium)), "other");
}

#[test]
fn parses_generic_and_qualified_types() {
    assert_eq!(pattern_speak(&Tagged { value: 7_u8 }), "lucky tag");
    assert_eq!(pattern_speak(&Tagged { value: 8_u8 }), "another tag");
    assert_eq!(pattern_speak(&nested::Otter), "otter");
}

#[test]
fn parses_comma_less_block_arms() {
    assert_eq!(comma_less_blocks(&Dog { size: Size::Small }), "dog");
    assert_eq!(comma_less_blocks(&Cat), "cat");
    assert_eq!(comma_less_blocks(&Cow), "other");
}

#[test]
fn guards_short_circuit_and_fall_through() {
    let calls = Cell::new(0);
    assert_eq!(guarded_fallback(&Cat, &calls), "guarded fallback");
    assert_eq!(calls.get(), 0, "a Dog guard must not run for a Cat");

    assert_eq!(
        guarded_fallback(&Dog { size: Size::Small }, &calls),
        "final fallback"
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn evaluates_the_match_value_once() {
    let calls = Cell::new(0);
    let cat = Cat;
    assert_eq!(count_evaluations(&cat, &calls).speak(), "meow");
    assert_eq!(calls.get(), 1);
}

#[test]
fn adapts_a_trait_that_cannot_be_modified() {
    assert_eq!(instrument_name(&foreign::Bell), "bell");
    assert_eq!(instrument_name(&foreign::Drum), "boom");
}
