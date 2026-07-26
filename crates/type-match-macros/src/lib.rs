use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    Expr, Ident, ItemTrait, Pat, PathArguments, Result, Token, Type, TypeParamBound, TypePath,
    Visibility, braced, parse_macro_input,
};

/// Marks a trait as usable with `type_match!`.
///
/// # Syntax
///
/// Place the attribute on an object-safe trait that you own:
///
/// ```ignore
/// use type_match::downcastable;
///
/// #[downcastable]
/// trait Animal {
///     fn speak(&self) -> &'static str;
/// }
/// ```
///
/// The attribute adds `type_match::Downcast` as a supertrait while preserving
/// the trait's visibility, generics, methods, attributes, and source spans.
/// Its effective expansion is:
///
/// ```ignore
/// trait Animal: type_match::Downcast {
///     fn speak(&self) -> &'static str;
/// }
/// ```
///
/// Implementations do not need to define downcast methods. `Downcast` has a
/// blanket implementation for every sized `'static` type.
///
/// For a trait owned by another crate, use [`downcastable_adapter`] instead.
#[proc_macro_attribute]
pub fn downcastable(_attribute: TokenStream, item: TokenStream) -> TokenStream {
    let mut item = parse_macro_input!(item as ItemTrait);
    let crate_path = runtime_crate_path();
    let bound = syn::parse2::<TypeParamBound>(quote!(#crate_path::Downcast))
        .expect("generated Downcast bound must parse");
    item.supertraits.push(bound);
    quote!(#item).into()
}

struct Adapter {
    visibility: Visibility,
    name: Ident,
    bounds: Punctuated<TypeParamBound, Token![+]>,
}

impl Parse for Adapter {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let visibility = input.parse()?;
        input.parse::<Token![trait]>()?;
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        let bounds = Punctuated::parse_separated_nonempty(input)?;
        input.parse::<Option<Token![;]>>()?;
        Ok(Self {
            visibility,
            name,
            bounds,
        })
    }
}

/// Generates a downcastable local facade for a trait that cannot be modified.
///
/// # Syntax
///
/// ```ignore
/// use type_match::downcastable_adapter;
///
/// downcastable_adapter! {
///     pub trait LocalFacade: path::to::ForeignTrait;
/// }
/// ```
///
/// Visibility is optional, the local facade name follows `trait`, and one or
/// more `+`-separated foreign bounds follow `:`:
///
/// ```ignore
/// downcastable_adapter! {
///     pub(crate) trait MatchWidget: foreign::Widget + Send + Sync;
/// }
/// ```
///
/// The macro generates an empty local trait extending the requested bounds
/// and `type_match::Downcast`, plus a blanket implementation for every
/// compatible concrete type. Accept `&dyn LocalFacade` at the matching
/// boundary instead of `&dyn ForeignTrait`.
///
/// This cannot retrofit an existing `&dyn ForeignTrait`: once a value has
/// already been erased behind a vtable without an `Any` projection, safe Rust
/// cannot recover its concrete type.
#[proc_macro]
pub fn downcastable_adapter(input: TokenStream) -> TokenStream {
    let Adapter {
        visibility,
        name,
        bounds,
    } = parse_macro_input!(input as Adapter);
    let crate_path = runtime_crate_path();
    let implementation = format_ident!("__TypeMatchImplementation", span = Span::mixed_site());

    quote! {
        #visibility trait #name: #bounds + #crate_path::Downcast {}

        impl<#implementation> #name for #implementation
        where
            #implementation: #bounds + #crate_path::Downcast,
        {}
    }
    .into()
}

struct TypeMatch {
    value: Expr,
    arms: Vec<Arm>,
}

struct Arm {
    kind: ArmKind,
    guard: Option<Expr>,
    body: Expr,
}

enum ArmKind {
    Type {
        binding: Option<Ident>,
        ty: Box<Type>,
    },
    Pattern {
        binding: Option<Ident>,
        pattern: Box<Pat>,
        ty: Box<Type>,
    },
    Wildcard,
    Other(Ident),
}

impl Parse for TypeMatch {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![match]>()?;
        let value = Expr::parse_without_eager_brace(input)?;
        let content;
        braced!(content in input);

        let mut arms: Vec<Arm> = Vec::new();
        while !content.is_empty() {
            arms.push(content.parse()?);
        }
        if arms.is_empty() {
            return Err(content.error("type_match! requires at least one arm"));
        }

        for (index, arm) in arms.iter().enumerate() {
            if matches!(arm.kind, ArmKind::Wildcard | ArmKind::Other(_))
                && arm.guard.is_none()
                && index + 1 != arms.len()
            {
                return Err(syn::Error::new_spanned(
                    &arm.body,
                    "an unguarded fallback must be the final arm",
                ));
            }
        }

        Ok(Self { value, arms })
    }
}

impl Parse for Arm {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let kind = if input.peek(Token![_]) {
            input.parse::<Token![_]>()?;
            ArmKind::Wildcard
        } else if is_other_fallback(input) {
            ArmKind::Other(input.parse()?)
        } else {
            let binding = parse_binding(input)?;
            if let Some((pattern, ty)) = parse_structured_pattern(input)? {
                ArmKind::Pattern {
                    binding,
                    pattern: Box::new(pattern),
                    ty: Box::new(ty),
                }
            } else {
                let ty = Box::new(input.parse()?);
                ArmKind::Type { binding, ty }
            }
        };

        let guard = if input.peek(Token![if]) {
            input.parse::<Token![if]>()?;
            Some(input.parse()?)
        } else {
            None
        };

        input.parse::<Token![=>]>()?;
        let body: Expr = input.parse()?;
        let has_comma = input.parse::<Option<Token![,]>>()?.is_some();
        if !has_comma && !input.is_empty() && !matches!(body, Expr::Block(_)) {
            return Err(input.error("non-block arms must be followed by a comma"));
        }

        Ok(Self { kind, guard, body })
    }
}

fn parse_structured_pattern(input: ParseStream<'_>) -> Result<Option<(Pat, Type)>> {
    let fork = input.fork();
    let Ok(pattern) = fork.call(Pat::parse_multi) else {
        return Ok(None);
    };
    if !fork.peek(Token![if]) && !fork.peek(Token![=>]) {
        return Ok(None);
    }

    let Some(ty) = pattern_type(&pattern) else {
        return Ok(None);
    };
    let pattern = input.call(Pat::parse_multi)?;
    Ok(Some((pattern, ty)))
}

fn pattern_type(pattern: &Pat) -> Option<Type> {
    let (qself, path) = match pattern {
        Pat::Struct(pattern) => (pattern.qself.clone(), pattern.path.clone()),
        Pat::TupleStruct(pattern) => (pattern.qself.clone(), pattern.path.clone()),
        _ => return None,
    };
    Some(Type::Path(TypePath { qself, path }))
}

fn pattern_with_inferred_generics(mut pattern: Box<Pat>) -> Box<Pat> {
    let path = match pattern.as_mut() {
        Pat::Struct(pattern) => &mut pattern.path,
        Pat::TupleStruct(pattern) => &mut pattern.path,
        _ => return pattern,
    };
    if let Some(segment) = path.segments.last_mut() {
        segment.arguments = PathArguments::None;
    }
    pattern
}

fn type_without_redundant_turbofish(mut ty: Box<Type>) -> Box<Type> {
    if let Type::Path(ty) = ty.as_mut() {
        for segment in &mut ty.path.segments {
            if let PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
                arguments.colon2_token = None;
            }
        }
    }
    ty
}

fn is_other_fallback(input: ParseStream<'_>) -> bool {
    if !input.peek(Ident) {
        return false;
    }
    let fork = input.fork();
    let Ok(ident) = fork.parse::<Ident>() else {
        return false;
    };
    ident == "other" && fork.peek(Token![=>])
}

fn parse_binding(input: ParseStream<'_>) -> Result<Option<Ident>> {
    if !input.peek(Ident) {
        return Ok(None);
    }
    let fork = input.fork();
    let ident: Ident = fork.parse()?;
    if fork.peek(Token![@]) {
        input.parse::<Ident>()?;
        input.parse::<Token![@]>()?;
        return Ok(Some(ident));
    }
    if fork.peek(Token![as]) {
        input.parse::<Ident>()?;
        input.parse::<Token![as]>()?;
        return Ok(Some(ident));
    }
    Ok(None)
}

/// Matches a downcastable trait object by concrete type and Rust patterns.
///
/// # Syntax
///
/// ```text
/// type_match! {
///     match VALUE {
///         Type => expression,
///         binding @ Type => expression,
///         binding as Type if guard => expression,
///
///         Struct { fields } => expression,
///         binding @ Struct { fields } if guard => expression,
///         TupleStruct(patterns...) => expression,
///
///         _ if guard => expression,
///         _ => fallback_expression,
///         other => bound_fallback_expression,
///     }
/// }
/// ```
///
/// `VALUE` is evaluated exactly once and must produce a shared trait-object
/// reference whose trait was prepared with [`downcastable`] or
/// [`downcastable_adapter`]. Arms are checked from top to bottom as one
/// `if / else if / else` chain.
///
/// # Type arms
///
/// `Type => body` tests the concrete type without binding it. Prefix the type
/// with `name @` or `name as` to bind the resulting `&Type`:
///
/// ```ignore
/// Dog => "a dog",
/// dog @ Dog => dog.speak(),
/// cat as Cat if cat.is_hungry() => cat.feed(),
/// ```
///
/// Qualified and generic types are accepted, such as `module::Dog` and
/// `Wrapper<u8>`.
///
/// # Structural pattern arms
///
/// Struct and tuple-struct patterns downcast and destructure in one `if let`:
///
/// ```ignore
/// Dog { size: Size::Small } => "small dog",
/// dog @ Dog { size } if size.is_large() => dog.speak(),
/// Pair(left, right) => combine(left, right),
/// Tagged::<u8> { value: 7 } => "lucky",
/// ```
///
/// In the last example, the generated pattern is `Tagged { value: 7 }` and
/// the downcast type is `Tagged<u8>`. Generic arguments are inferred in the
/// pattern and retained only in `downcast_ref::<Tagged<u8>>()`.
///
/// # Guards
///
/// Append `if expression` before `=>`. A guard runs only after its type and
/// structural pattern have matched. A failed guard continues to the next arm.
///
/// # Fallbacks
///
/// `_ => body` is an unbound fallback. `other => body` binds the original
/// trait-object reference, preserving access to its trait methods. An
/// unguarded fallback must be final; guarded `_` arms may occur earlier.
///
/// A missing fallback is allowed. Reaching the end then evaluates
/// `unreachable!("type_match!: no arm matched")`.
///
/// # Commas and blocks
///
/// Non-block expression arms require trailing commas unless final. Block arms
/// may omit commas, matching ordinary Rust `match` syntax.
///
/// # Return value
///
/// The macro is an expression. Every reachable arm body must resolve to a
/// compatible result type.
#[proc_macro]
pub fn type_match(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as TypeMatch)).into()
}

fn expand(input: TypeMatch) -> TokenStream2 {
    let crate_path = runtime_crate_path();
    let value = input.value;
    let value_ident = format_ident!("__type_match_value", span = Span::mixed_site());
    let label = syn::Lifetime::new("'__type_match", Span::mixed_site());

    let mut chain = quote!({ unreachable!("type_match!: no arm matched") });
    for arm in input.arms.into_iter().rev() {
        chain = expand_arm(arm, &crate_path, &value_ident, &label, chain);
    }

    quote! {{
        let #value_ident = #value;
        #[allow(unreachable_code, clippy::diverging_sub_expression)]
        #label: {
            #chain
        }
    }}
}

fn expand_arm(
    arm: Arm,
    crate_path: &TokenStream2,
    value: &Ident,
    label: &syn::Lifetime,
    next: TokenStream2,
) -> TokenStream2 {
    let Arm { kind, guard, body } = arm;
    match kind {
        ArmKind::Type {
            binding: Some(binding),
            ty,
        } => {
            let ty = type_without_redundant_turbofish(ty);
            let guard = expand_guard(guard);

            quote! {
                if let ::core::option::Option::Some(#binding) =
                    #crate_path::Downcast::as_any(#value).downcast_ref::<#ty>()
                    #guard
                {
                    break #label (#body);
                } else #next
            }
        }
        ArmKind::Type { binding: None, ty } => {
            let ty = type_without_redundant_turbofish(ty);
            let guard = expand_guard(guard);

            quote! {
                if #crate_path::Downcast::as_any(#value).is::<#ty>() #guard {
                    break #label (#body);
                } else #next
            }
        }
        ArmKind::Pattern {
            binding,
            pattern,
            ty,
        } => {
            let ty = type_without_redundant_turbofish(ty);
            let guard = expand_guard(guard);
            let pattern = pattern_with_inferred_generics(pattern);
            let pattern =
                binding.map_or_else(|| quote!(#pattern), |binding| quote!(#binding @ #pattern));

            quote! {
                if let ::core::option::Option::Some(#pattern) =
                    #crate_path::Downcast::as_any(#value).downcast_ref::<#ty>()
                    #guard
                {
                    break #label (#body);
                } else #next
            }
        }
        ArmKind::Wildcard => match guard {
            Some(guard) => quote! {
                if #guard {
                    break #label (#body);
                } else #next
            },
            None => quote!({ break #label (#body); }),
        },
        ArmKind::Other(binding) => quote!({
            let #binding = #value;
            break #label (#body);
        }),
    }
}

fn expand_guard(guard: Option<Expr>) -> TokenStream2 {
    if let Some(guard) = guard.map(|guard| quote!(#guard)) {
        quote!(&& (#guard))
    } else {
        quote!()
    }
}

fn runtime_crate_path() -> TokenStream2 {
    match crate_name("type-match") {
        Ok(FoundCrate::Itself) => quote!(::type_match),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            quote!(::#ident)
        }
        Err(_) => quote!(::type_match),
    }
}

#[cfg(test)]
mod tests {
    use super::{TypeMatch, expand};
    use quote::quote;

    #[test]
    fn parses_all_arm_shapes() {
        let input = quote! {
            match value {
                dog @ Dog { size: Size::Small } if dog.ready() => first(),
                cat as Cat => second(cat),
                Pair(left, right) => left + right,
                Wrapper::<u8> { value: 7 } => 7,
                module::Fox => fox(),
                _ if condition => guarded_fallback(),
                other => final_fallback(other),
            }
        };

        assert!(syn::parse2::<TypeMatch>(input).is_ok());
    }

    #[test]
    fn rejects_an_empty_match() {
        assert!(syn::parse2::<TypeMatch>(quote!(match value {})).is_err());
    }

    #[test]
    fn rejects_a_non_final_unguarded_fallback() {
        let input = quote! {
            match value {
                _ => fallback(),
                Dog => dog(),
            }
        };
        assert!(syn::parse2::<TypeMatch>(input).is_err());
    }

    #[test]
    fn rejects_a_missing_expression_comma() {
        let input = quote! {
            match value {
                Dog => dog()
                Cat => cat()
            }
        };
        assert!(syn::parse2::<TypeMatch>(input).is_err());
    }

    #[test]
    fn expansion_chains_arms_and_infers_pattern_generics() {
        let input = syn::parse2::<TypeMatch>(quote! {
            match value {
                Tagged::<u8> { value: 7 } => "lucky",
                Other => "other",
                _ => "fallback",
            }
        })
        .expect("test input should parse");
        let expansion = expand(input).to_string().replace(' ', "");

        assert!(expansion.contains("}elseif"), "{expansion}");
        assert!(expansion.contains("Some(Tagged{value:7})"), "{expansion}");
        assert!(
            expansion.contains("downcast_ref::<Tagged<u8>>()"),
            "{expansion}"
        );
        assert!(!expansion.contains("Some(Tagged::<u8>{"), "{expansion}");
        assert!(!expansion.contains("&&(true)"), "{expansion}");
        assert!(!expansion.contains("&&()"), "{expansion}");
    }

    #[test]
    fn expansion_only_emits_a_guard_suffix_when_present() {
        let input = syn::parse2::<TypeMatch>(quote! {
            match value {
                Dog if check() => "guarded",
                Cat => "unguarded",
                _ => "fallback",
            }
        })
        .expect("test input should parse");
        let expansion = expand(input).to_string().replace(' ', "");

        assert_eq!(expansion.matches("&&(check())").count(), 1, "{expansion}");
        assert!(!expansion.contains("&&(true)"), "{expansion}");
        assert!(!expansion.contains("&&()"), "{expansion}");
    }
}
