use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, parse_macro_input};

/// Derives [`geotrace_sdk::EventKind`] for an enum.
///
/// Each variant's name is converted to `snake_case` and becomes one segment of
/// a slash-separated path string.  Nesting enums that all derive `EventKind`
/// produces paths like `"power/boot"` or `"connectivity/agps/request"`.
///
/// # Enum-level attributes
///
/// Place these on the enum itself.  Multiple can be combined in one attribute:
/// `#[event_kind(lax, note = display)]`.
///
/// - *(none)* / **`#[event_kind(strict)]`** - single-field tuple variants
///   **delegate** to the inner type by default. A compile error is raised if
///   the inner type does not implement `EventKind`.  This is the default.
/// - **`#[event_kind(lax)]`** - single-field tuple variants are **leaves** by
///   default (emit only their own segment, no delegation).  Use
///   `#[event_kind(delegate)]` on individual variants to opt into delegation.
/// - **`#[event_kind(note = debug)]`** *(default)* - `event_note()` returns
///   `Some(format!("{self:?}"))`.  The type must implement `Debug`.
/// - **`#[event_kind(note = display)]`** - `event_note()` returns
///   `Some(format!("{self}"))`.  The type must implement `Display`.
/// - **`#[event_kind(note = none)]`** - `event_note()` always returns `None`.
///
/// # Variant-level attributes
///
/// - **`#[event_kind(leaf)]`** - always emit only this variant's segment.
///   Never delegate to the inner type even if it implements `EventKind`.
/// - **`#[event_kind(delegate)]`** - always delegate to the inner type's
///   `EventKind` implementation, appending its path after this variant's
///   segment.  In `lax` enum mode this opts a single variant into delegation.
/// - **`#[event_kind(skip)]`** - `variant_path()` returns `None` for this
///   variant. [`NavRecorder::add_event`](geotrace_sdk::NavRecorder::add_event)
///   silently ignores it.
/// - **`#[event_kind(icon = <Name>)]`** - sets the
///   [`MarkerIcon`](geotrace_sdk::MarkerIcon) for this variant (e.g.
///   `#[event_kind(icon = Warning)]`).  Attributes can be combined:
///   `#[event_kind(leaf, icon = Check)]`.  Has no effect on delegating variants
///   - their icon is taken from the inner type's leaf.
#[proc_macro_derive(EventKind, attributes(event_kind))]
pub fn derive_event_kind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_impl(&input).unwrap_or_else(|e| e.to_compile_error().into())
}

fn derive_impl(input: &DeriveInput) -> Result<TokenStream, syn::Error> {
    let EnumAttrs {
        mode: enum_mode,
        note: note_mode,
    } = parse_enum_attrs(&input.attrs)?;

    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[derive(EventKind)] is only supported on enums",
        ));
    };

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let results: Result<Vec<(TokenStream2, TokenStream2)>, syn::Error> = data
        .variants
        .iter()
        .map(|v| generate_arm(v, &enum_mode))
        .collect();
    let results = results?;
    let (path_arms, icon_arms): (Vec<_>, Vec<_>) = results.into_iter().unzip();

    let note_body = match note_mode {
        EnumNoteMode::Debug => quote! {
            ::core::option::Option::Some(::std::format!("{:?}", self))
        },
        EnumNoteMode::Display => quote! {
            ::core::option::Option::Some(::std::format!("{}", self))
        },
        EnumNoteMode::None => quote! {
            ::core::option::Option::None
        },
    };

    Ok(quote! {
        impl #impl_generics ::geotrace_sdk::__private::Sealed for #name #ty_generics #where_clause {}

        impl #impl_generics ::geotrace_sdk::EventKind for #name #ty_generics #where_clause {
            fn variant_path(&self) -> ::core::option::Option<::std::string::String> {
                match self {
                    #(#path_arms)*
                }
            }

            fn marker_icon(&self) -> ::core::option::Option<::geotrace_sdk::MarkerIcon> {
                match self {
                    #(#icon_arms)*
                }
            }

            fn event_note(&self) -> ::core::option::Option<::std::string::String> {
                #note_body
            }
        }
    }
    .into())
}

#[derive(Debug)]
enum EnumMode {
    Strict,
    Lax,
}

#[derive(Debug)]
enum EnumNoteMode {
    Debug,
    Display,
    None,
}

#[derive(Debug)]
struct EnumAttrs {
    mode: EnumMode,
    note: EnumNoteMode,
}

#[derive(Debug)]
enum VariantMode {
    Default,
    Leaf,
    Delegate,
    Skip,
}

fn parse_enum_attrs(attrs: &[Attribute]) -> Result<EnumAttrs, syn::Error> {
    let mut mode = EnumMode::Strict;
    let mut note = EnumNoteMode::Debug;

    for attr in attrs {
        if !attr.path().is_ident("event_kind") {
            continue;
        }
        let nested = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        )?;
        for meta in &nested {
            match meta {
                syn::Meta::Path(p) if p.is_ident("strict") => {
                    mode = EnumMode::Strict;
                }
                syn::Meta::Path(p) if p.is_ident("lax") => {
                    mode = EnumMode::Lax;
                }
                syn::Meta::NameValue(nv) if nv.path.is_ident("note") => {
                    if let syn::Expr::Path(ep) = &nv.value {
                        if let Some(ident) = ep.path.get_ident() {
                            note = match ident.to_string().as_str() {
                                "debug" => EnumNoteMode::Debug,
                                "display" => EnumNoteMode::Display,
                                "none" => EnumNoteMode::None,
                                other => {
                                    return Err(syn::Error::new_spanned(
                                        ident,
                                        format!(
                                            "unknown note mode {other:?}; expected one of: debug, display, none"
                                        ),
                                    ));
                                }
                            };
                        } else {
                            return Err(syn::Error::new_spanned(
                                &nv.value,
                                "expected one of: debug, display, none",
                            ));
                        }
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "expected one of: debug, display, none",
                        ));
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unknown event_kind enum attribute; expected one of: strict, lax, note = <debug|display|none>",
                    ));
                }
            }
        }
    }

    Ok(EnumAttrs { mode, note })
}

fn parse_variant_attrs(
    attrs: &[Attribute],
) -> Result<(VariantMode, Option<syn::Ident>), syn::Error> {
    let mut mode = VariantMode::Default;
    let mut icon = None;

    for attr in attrs {
        if !attr.path().is_ident("event_kind") {
            continue;
        }
        let nested = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        )?;
        for meta in &nested {
            match meta {
                syn::Meta::Path(p) if p.is_ident("leaf") || p.is_ident("lax") => {
                    mode = VariantMode::Leaf;
                }
                syn::Meta::Path(p) if p.is_ident("delegate") => {
                    mode = VariantMode::Delegate;
                }
                syn::Meta::Path(p) if p.is_ident("skip") => {
                    mode = VariantMode::Skip;
                }
                syn::Meta::NameValue(nv) if nv.path.is_ident("icon") => {
                    if let syn::Expr::Path(expr_path) = &nv.value {
                        if let Some(ident) = expr_path.path.get_ident() {
                            icon = Some(ident.clone());
                        } else {
                            return Err(syn::Error::new_spanned(
                                &nv.value,
                                "expected a simple icon name like `Pin` or `Warning`",
                            ));
                        }
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "expected a simple icon name like `Pin` or `Warning`",
                        ));
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unknown event_kind attribute; expected one of: leaf, delegate, skip, icon = <Name>",
                    ));
                }
            }
        }
    }

    Ok((mode, icon))
}

fn generate_arm(
    variant: &syn::Variant,
    enum_mode: &EnumMode,
) -> Result<(TokenStream2, TokenStream2), syn::Error> {
    let (mode, icon_ident) = parse_variant_attrs(&variant.attrs)?;
    let ident = &variant.ident;
    let seg = to_snake_case(&ident.to_string());

    let icon_expr = match &icon_ident {
        Some(name) => quote! { ::core::option::Option::Some(::geotrace_sdk::MarkerIcon::#name) },
        None => quote! { ::core::option::Option::None },
    };

    if matches!(mode, VariantMode::Skip) {
        let pat = wildcard_pat(variant);
        return Ok((
            quote! { #pat => ::core::option::Option::None, },
            quote! { #pat => ::core::option::Option::None, },
        ));
    }

    if matches!(mode, VariantMode::Leaf) {
        let pat = wildcard_pat(variant);
        return Ok((
            quote! { #pat => ::core::option::Option::Some(::std::string::String::from(#seg)), },
            quote! { #pat => #icon_expr, },
        ));
    }

    match &variant.fields {
        Fields::Unit => Ok((
            quote! {
                Self::#ident => ::core::option::Option::Some(::std::string::String::from(#seg)),
            },
            quote! {
                Self::#ident => #icon_expr,
            },
        )),

        Fields::Unnamed(f) if f.unnamed.len() == 1 => {
            let should_delegate =
                matches!(mode, VariantMode::Delegate) || matches!(enum_mode, EnumMode::Strict);

            if should_delegate {
                Ok((
                    quote! {
                        Self::#ident(inner) => {
                            let inner_path = ::geotrace_sdk::EventKind::variant_path(inner)?;
                            ::core::option::Option::Some(
                                ::std::format!("{}/{}", #seg, inner_path)
                            )
                        },
                    },
                    quote! {
                        Self::#ident(inner) => ::geotrace_sdk::EventKind::marker_icon(inner),
                    },
                ))
            } else {
                let pat = wildcard_pat(variant);
                Ok((
                    quote! {
                        #pat => ::core::option::Option::Some(::std::string::String::from(#seg)),
                    },
                    quote! {
                        #pat => #icon_expr,
                    },
                ))
            }
        }

        Fields::Unnamed(_) | Fields::Named(_) => {
            let pat = wildcard_pat(variant);
            Ok((
                quote! {
                    #pat => ::core::option::Option::Some(::std::string::String::from(#seg)),
                },
                quote! {
                    #pat => #icon_expr,
                },
            ))
        }
    }
}

fn wildcard_pat(variant: &syn::Variant) -> TokenStream2 {
    let ident = &variant.ident;
    match &variant.fields {
        Fields::Unit => quote! { Self::#ident },
        Fields::Unnamed(_) => quote! { Self::#ident(..) },
        Fields::Named(_) => quote! { Self::#ident { .. } },
    }
}

fn to_snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                let prev = chars.get(i - 1).copied().unwrap_or('\0');
                let next = chars.get(i + 1).copied();
                let next_is_lower = next.is_some_and(|n| n.is_lowercase() || n.is_ascii_digit());
                if prev.is_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_uppercase() && next_is_lower)
                {
                    result.push('_');
                }
            }
            for lc in c.to_lowercase() {
                result.push(lc);
            }
        } else {
            result.push(c);
        }
    }
    result
}
