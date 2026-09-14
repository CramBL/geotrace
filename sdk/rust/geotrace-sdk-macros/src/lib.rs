use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::attributes::{EnumAttributes, EnumNoteMode};
use crate::derived_variant::DerivedVariant;

mod attributes;
mod derived_variant;
mod variant_path_segment;

/// Derives `geotrace_sdk::EventKind` for an `enum`.
///
/// Each variant's name becomes one segment of a slash-separated path string, in `snake_case`.
/// A run of capitals is one word, apart from a last capital before a lower-case letter:
/// `GPS3Lock` gives `gps3_lock` and `HTTPError` gives `http_error`. A raw identifier loses its
/// `r#` (`r#type` gives `type`). Nested `enum` types that all derive `EventKind` produce paths
/// like `"power/boot"` or `"connectivity/agps/request"`.
///
/// A segment has only ASCII letters, digits, `-` and `_`, and at most 255 bytes, and no two
/// variants of one `enum` have the same segment. The derive reports a compile error on a variant
/// that breaks either rule, and `rename` sets another segment for it. A nested path past 255
/// bytes makes `geotrace_sdk::NavRecorder::finish` fail.
///
/// # Enum-level attributes
///
/// Place these on the `enum` itself.  Multiple can be combined in one attribute:
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
///   segment.  In `lax` mode this opts a single variant into delegation. Only a
///   variant with one unnamed field delegates.
/// - **`#[event_kind(skip)]`** - `variant_path()` returns `None` for this
///   variant. `geotrace_sdk::NavRecorder::add_event`
///   silently ignores it.
/// - **`#[event_kind(icon = <Name>)]`** - sets the
///   `geotrace_sdk::MarkerIcon` for this variant (e.g.
///   `#[event_kind(icon = Warning)]`).  Attributes can be combined:
///   `#[event_kind(leaf, icon = Check)]`.  A delegating variant takes its icon
///   from the inner type's leaf.
/// - **`#[event_kind(rename = "<segment>")]`** - sets this variant's segment in
///   place of the one derived from its name.
///
/// The derive reports a compile error for an attribute without effect on its
/// variant, such as `icon` on a skipped or a delegating variant, and for a second
/// attribute of one kind, such as `leaf` after `delegate`, `lax` after `strict` or
/// a second `note`.
#[proc_macro_derive(EventKind, attributes(event_kind))]
pub fn derive_event_kind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_impl(&input).unwrap_or_else(|e| e.to_compile_error().into())
}

fn derive_impl(input: &DeriveInput) -> Result<TokenStream, syn::Error> {
    let EnumAttributes { mode, note } = EnumAttributes::parse(&input.attrs)?;

    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[derive(EventKind)] is only supported on enums",
        ));
    };

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let variants = DerivedVariant::derive_all_with_distinct_segments(data, mode)?;
    let path_arms = variants.iter().map(DerivedVariant::variant_path_arm);
    let icon_arms = variants.iter().map(DerivedVariant::marker_icon_arm);

    let note_body = match note {
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
