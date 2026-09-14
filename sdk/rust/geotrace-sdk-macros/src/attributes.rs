use quote::ToTokens as _;
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, ExprLit, Ident, Lit, LitStr, Meta, Token};

pub(crate) struct EnumAttributes {
    pub(crate) mode: EnumMode,
    pub(crate) note: EnumNoteMode,
}

impl EnumAttributes {
    pub(crate) fn parse(attrs: &[Attribute]) -> Result<Self, syn::Error> {
        let mut mode = None;
        let mut note = None;
        for meta in event_kind_metas(attrs)? {
            match &meta {
                Meta::Path(path) if path.is_ident("lax") => {
                    AttributeSetting::set_once(&mut mode, EnumMode::Lax, &meta)?;
                }
                Meta::Path(path) if path.is_ident("strict") => {
                    AttributeSetting::set_once(&mut mode, EnumMode::Strict, &meta)?;
                }
                Meta::NameValue(name_value) if name_value.path.is_ident("note") => {
                    let note_mode = EnumNoteMode::parse(&name_value.value)?;
                    AttributeSetting::set_once(&mut note, note_mode, &meta)?;
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unknown event_kind enum attribute; expected one of: strict, lax, note = <debug|display|none>",
                    ));
                }
            }
        }
        Ok(Self {
            mode: mode.map_or(EnumMode::Strict, |setting| setting.value),
            note: note.map_or(EnumNoteMode::Debug, |setting| setting.value),
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum EnumMode {
    Lax,
    Strict,
}

pub(crate) enum EnumNoteMode {
    Debug,
    Display,
    None,
}

impl EnumNoteMode {
    fn parse(value: &Expr) -> Result<Self, syn::Error> {
        let expected_note_mode =
            || syn::Error::new_spanned(value, "expected one of: debug, display, none");
        let Expr::Path(expr_path) = value else {
            return Err(expected_note_mode());
        };
        let Some(ident) = expr_path.path.get_ident() else {
            return Err(expected_note_mode());
        };
        match ident.to_string().as_str() {
            "debug" => Ok(Self::Debug),
            "display" => Ok(Self::Display),
            "none" => Ok(Self::None),
            other => Err(syn::Error::new_spanned(
                ident,
                format!("unknown note mode {other:?}; expected one of: debug, display, none"),
            )),
        }
    }
}

pub(crate) struct VariantAttributes {
    pub(crate) mode: Option<AttributeSetting<VariantMode>>,
    pub(crate) icon: Option<AttributeSetting<Ident>>,
    pub(crate) rename: Option<AttributeSetting<LitStr>>,
}

impl VariantAttributes {
    pub(crate) fn parse(attrs: &[Attribute]) -> Result<Self, syn::Error> {
        let mut mode = None;
        let mut icon = None;
        let mut rename = None;
        for meta in event_kind_metas(attrs)? {
            match &meta {
                Meta::Path(path) if path.is_ident("delegate") => {
                    AttributeSetting::set_once(&mut mode, VariantMode::Delegate, &meta)?;
                }
                Meta::Path(path) if path.is_ident("leaf") || path.is_ident("lax") => {
                    AttributeSetting::set_once(&mut mode, VariantMode::Leaf, &meta)?;
                }
                Meta::Path(path) if path.is_ident("skip") => {
                    AttributeSetting::set_once(&mut mode, VariantMode::Skip, &meta)?;
                }
                Meta::NameValue(name_value) if name_value.path.is_ident("icon") => {
                    let icon_name = parse_icon_name(&name_value.value)?;
                    AttributeSetting::set_once(&mut icon, icon_name, &meta)?;
                }
                Meta::NameValue(name_value) if name_value.path.is_ident("rename") => {
                    let Expr::Lit(ExprLit {
                        lit: Lit::Str(segment),
                        ..
                    }) = &name_value.value
                    else {
                        return Err(syn::Error::new_spanned(
                            &name_value.value,
                            "expected a string literal like \"gps_lock\"",
                        ));
                    };
                    AttributeSetting::set_once(&mut rename, segment.clone(), &meta)?;
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unknown event_kind attribute; expected one of: leaf, delegate, skip, icon = <Name>, rename = \"<segment>\"",
                    ));
                }
            }
        }
        Ok(Self { mode, icon, rename })
    }
}

pub(crate) enum VariantMode {
    Delegate,
    Leaf,
    Skip,
}

/// The value of one `event_kind` attribute, with the attribute itself for the span and the text
/// of a compile error.
pub(crate) struct AttributeSetting<T> {
    pub(crate) value: T,
    pub(crate) meta: Meta,
}

impl<T> AttributeSetting<T> {
    fn set_once(slot: &mut Option<Self>, value: T, meta: &Meta) -> Result<(), syn::Error> {
        if let Some(earlier) = slot {
            return Err(syn::Error::new_spanned(
                meta,
                format!(
                    "`{}` conflicts with the earlier `{}`",
                    meta.to_token_stream(),
                    earlier.meta.to_token_stream()
                ),
            ));
        }
        *slot = Some(Self {
            value,
            meta: meta.clone(),
        });
        Ok(())
    }
}

fn event_kind_metas(attrs: &[Attribute]) -> Result<Vec<Meta>, syn::Error> {
    let mut metas = Vec::new();
    for attr in attrs
        .iter()
        .filter(|attr| attr.path().is_ident("event_kind"))
    {
        metas.extend(attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?);
    }
    Ok(metas)
}

fn parse_icon_name(value: &Expr) -> Result<Ident, syn::Error> {
    match value {
        Expr::Path(expr_path) => expr_path.path.get_ident().cloned(),
        _ => None,
    }
    .ok_or_else(|| {
        syn::Error::new_spanned(value, "expected a simple icon name like `Pin` or `Warning`")
    })
}
