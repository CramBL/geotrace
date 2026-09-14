use std::collections::BTreeMap;

use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens as _, quote};
use syn::{DataEnum, Fields, Ident, LitStr, Variant};

use crate::attributes::{AttributeSetting, EnumMode, VariantAttributes, VariantMode};
use crate::variant_path_segment::VariantPathSegment;

pub(crate) struct DerivedVariant<'a> {
    variant: &'a Variant,
    kind: VariantKind,
}

enum VariantKind {
    Delegate {
        segment: VariantPathSegment,
    },
    Leaf {
        segment: VariantPathSegment,
        icon: Option<Ident>,
    },
    Skip,
}

impl<'a> DerivedVariant<'a> {
    fn segment_of(
        variant_name: &Ident,
        rename: Option<LitStr>,
    ) -> Result<VariantPathSegment, syn::Error> {
        match rename {
            Some(rename) => VariantPathSegment::try_from(rename.value())
                .map_err(|error| syn::Error::new_spanned(&rename, error)),
            None => VariantPathSegment::from_variant_name(variant_name).map_err(|error| {
                syn::Error::new_spanned(variant_name, format!("{error}: {SET_A_SEGMENT}"))
            }),
        }
    }

    pub(crate) fn derive_all_with_distinct_segments(
        data: &'a DataEnum,
        enum_mode: EnumMode,
    ) -> Result<Vec<Self>, syn::Error> {
        let mut variant_by_segment: BTreeMap<String, &Ident> = BTreeMap::new();
        let mut derived_variants = Vec::with_capacity(data.variants.len());
        for variant in &data.variants {
            let derived_variant = Self::derive(variant, enum_mode)?;
            if let Some(segment) = derived_variant.segment()
                && let Some(earlier) =
                    variant_by_segment.insert(segment.as_ref().to_owned(), &variant.ident)
            {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    format!(
                        "variants `{earlier}` and `{}` both have the variant path segment {:?}: {SET_A_SEGMENT}",
                        variant.ident,
                        segment.as_ref(),
                    ),
                ));
            }
            derived_variants.push(derived_variant);
        }
        Ok(derived_variants)
    }

    fn derive(variant: &'a Variant, enum_mode: EnumMode) -> Result<Self, syn::Error> {
        let VariantAttributes { mode, icon, rename } = VariantAttributes::parse(&variant.attrs)?;
        let has_one_unnamed_field =
            matches!(&variant.fields, Fields::Unnamed(fields) if fields.unnamed.len() == 1);
        let delegates = match mode {
            None => has_one_unnamed_field && enum_mode == EnumMode::Strict,
            Some(AttributeSetting {
                value: VariantMode::Delegate,
                ..
            }) if has_one_unnamed_field => true,
            Some(AttributeSetting {
                value: VariantMode::Delegate,
                meta,
            }) => {
                return Err(syn::Error::new_spanned(
                    meta,
                    "`delegate` needs a variant with one unnamed field, such as `Power(PowerEvent)`",
                ));
            }
            Some(AttributeSetting {
                value: VariantMode::Leaf,
                ..
            }) => false,
            Some(AttributeSetting {
                value: VariantMode::Skip,
                ..
            }) => return Self::skipped(variant, icon, rename),
        };
        let segment = Self::segment_of(&variant.ident, rename.map(|setting| setting.value))?;
        let kind = match (delegates, icon) {
            (true, Some(AttributeSetting { meta, .. })) => {
                return Err(syn::Error::new_spanned(
                    &meta,
                    format!(
                        "`{}` has no effect on a delegating variant: set the icon on the variant of the inner event",
                        meta.to_token_stream()
                    ),
                ));
            }
            (true, None) => VariantKind::Delegate { segment },
            (false, icon) => VariantKind::Leaf {
                segment,
                icon: icon.map(|setting| setting.value),
            },
        };
        Ok(Self { variant, kind })
    }

    fn skipped(
        variant: &'a Variant,
        icon: Option<AttributeSetting<Ident>>,
        rename: Option<AttributeSetting<LitStr>>,
    ) -> Result<Self, syn::Error> {
        let attribute_without_effect = icon
            .map(|setting| setting.meta)
            .or_else(|| rename.map(|setting| setting.meta));
        if let Some(meta) = attribute_without_effect {
            return Err(syn::Error::new_spanned(
                &meta,
                format!(
                    "`{}` has no effect on a skipped variant",
                    meta.to_token_stream()
                ),
            ));
        }
        Ok(Self {
            variant,
            kind: VariantKind::Skip,
        })
    }

    pub(crate) fn variant_path_arm(&self) -> TokenStream2 {
        let ident = &self.variant.ident;
        let pattern = self.wildcard_pattern();
        match &self.kind {
            VariantKind::Delegate { segment } => quote! {
                Self::#ident(inner) => {
                    let inner_path = ::geotrace_sdk::EventKind::variant_path(inner)?;
                    ::core::option::Option::Some(::std::format!("{}/{}", #segment, inner_path))
                },
            },
            VariantKind::Leaf { segment, .. } => quote! {
                #pattern => ::core::option::Option::Some(::std::string::String::from(#segment)),
            },
            VariantKind::Skip => quote! {
                #pattern => ::core::option::Option::None,
            },
        }
    }

    pub(crate) fn marker_icon_arm(&self) -> TokenStream2 {
        let ident = &self.variant.ident;
        let pattern = self.wildcard_pattern();
        match &self.kind {
            VariantKind::Delegate { .. } => quote! {
                Self::#ident(inner) => ::geotrace_sdk::EventKind::marker_icon(inner),
            },
            VariantKind::Leaf {
                icon: Some(icon), ..
            } => quote! {
                #pattern => ::core::option::Option::Some(::geotrace_sdk::MarkerIcon::#icon),
            },
            VariantKind::Leaf { icon: None, .. } | VariantKind::Skip => quote! {
                #pattern => ::core::option::Option::None,
            },
        }
    }

    fn segment(&self) -> Option<&VariantPathSegment> {
        match &self.kind {
            VariantKind::Delegate { segment } | VariantKind::Leaf { segment, .. } => Some(segment),
            VariantKind::Skip => None,
        }
    }

    fn wildcard_pattern(&self) -> TokenStream2 {
        let ident = &self.variant.ident;
        match &self.variant.fields {
            Fields::Unit => quote! { Self::#ident },
            Fields::Unnamed(_) => quote! { Self::#ident(..) },
            Fields::Named(_) => quote! { Self::#ident { .. } },
        }
    }
}

const SET_A_SEGMENT: &str = "set another with #[event_kind(rename = \"<segment>\")]";
