use proc_macro2::TokenStream as TokenStream2;
use quote::{
    format_ident,
    quote_spanned,
};
use syn::spanned::Spanned as _;

pub fn generate(input: TokenStream2) -> TokenStream2 {
    match generate_or_error(input) {
        Ok(output) => output,
        Err(err) => err.to_compile_error(),
    }
}

fn generate_or_error(input: TokenStream2) -> syn::Result<TokenStream2> {
    let input = syn::parse::<syn::DeriveInput>(input.into())?;
    match input.data {
        syn::Data::Enum(data_enum) => {
            generate_enum(syn::ItemEnum {
                attrs: input.attrs,
                vis: input.vis,
                enum_token: data_enum.enum_token,
                ident: input.ident,
                generics: input.generics,
                brace_token: data_enum.brace_token,
                variants: data_enum.variants,
            })
        }
        syn::Data::Struct(_) => {
            Err(format_err!(
                input,
                "structs are not supported as bitfield specifiers",
            ))
        }
        syn::Data::Union(_) => {
            Err(format_err!(
                input,
                "unions are not supported as bitfield specifiers",
            ))
        }
    }
}

struct Attributes {
    bits: Option<usize>,
}

fn parse_attrs(attrs: &[syn::Attribute]) -> syn::Result<Attributes> {
    let attributes = attrs
        .iter()
        .filter(|attr| attr.path.is_ident("bits"))
        .fold(
            Ok(Attributes { bits: None }),
            |acc: syn::Result<Attributes>, attr| {
                let mut acc = acc?;
                if acc.bits.is_some() {
                    return Err(format_err_spanned!(
                        attr,
                        "More than one 'bits' attributes is not permitted",
                    ))
                }
                let meta = attr.parse_meta()?;
                acc.bits = match meta {
                    syn::Meta::NameValue(syn::MetaNameValue {
                        lit: syn::Lit::Int(lit),
                        ..
                    }) => Some(lit.base10_parse::<usize>()?),
                    _ => {
                        return Err(format_err_spanned!(
                            attr,
                            "could not parse 'bits' attribute",
                        ))
                    }
                };
                Ok(acc)
            },
        )?;
    Ok(attributes)
}

pub fn generate_enum(input: syn::ItemEnum) -> syn::Result<TokenStream2> {
    let attributes = parse_attrs(&input.attrs)?;
    let enum_ident = &input.ident;

    let (bits, _returns_result) = match attributes.bits {
        Some(bits) => (bits, true),
        None => {
            let count_variants = input.variants.iter().count();
            if !count_variants.is_power_of_two() {
                return Err(format_err_spanned!(
                    input,
                    "A BitfieldSpecifier with a non-power-of-2 number of variants must have the 'bits' attribute specified",
                ));
            }
            // We can take `trailing_zeros` returns type as the required amount of bits.
            (count_variants.trailing_zeros() as usize, false)
        }
    };

    let variants = input
        .variants
        .iter()
        .filter_map(|variant| {
            match &variant.fields {
                syn::Fields::Unit => Some(&variant.ident),
                _ => None,
            }
        })
        .collect::<Vec<_>>();

    let check_discriminants = variants.iter().map(|ident| {
        let span = ident.span();
        quote_spanned!(span =>
            impl ::modular_bitfield::private::checks::CheckDiscriminantInRange<[(); Self::#ident as usize]> for #enum_ident {
                type CheckType = [(); ((Self::#ident as usize) < (0x01_usize << #bits)) as usize ];
            }
        )
    });
    let match_arms = variants.iter().map(|ident| {
        use heck::SnakeCase as _;
        let span = ident.span();
        let snake_variant = &ident.to_string().to_snake_case();
        let snake_variant = match syn::parse_str::<syn::Ident>(snake_variant) {
            Ok(parsed_ident) => parsed_ident,
            // Use a raw identifier to allow strict keywords.
            Err(_) => format_ident!("r#{}", snake_variant),
        };
        quote_spanned!(span=>
            #snake_variant if #snake_variant == Self::#ident as <Self as ::modular_bitfield::Specifier>::Base => {
                Self::#ident
            }
        )
    });

    Ok(quote_spanned!(input.span() =>
        #( #check_discriminants )*

        impl ::modular_bitfield::Specifier for #enum_ident {
            const BITS: usize = #bits;
            type Base = <[(); #bits] as ::modular_bitfield::private::SpecifierBase>::Base;
            type Face = Self;
        }

        impl ::modular_bitfield::private::FromBits<<Self as ::modular_bitfield::Specifier>::Base> for #enum_ident {
            #[inline(always)]
            fn from_bits(bits: ::modular_bitfield::private::Bits<<Self as ::modular_bitfield::Specifier>::Base>) -> Self {
                match bits.into_raw() {
                    #( #match_arms )*
                    // This API is only used internally and is only invoked on valid input.
                    // Thus it is find to omit error handling for cases where the incoming
                    // value is out of bounds to improve performance.
                    _ => { unsafe { ::core::hint::unreachable_unchecked() } }
                }
            }
        }

        impl ::modular_bitfield::private::IntoBits<<Self as ::modular_bitfield::Specifier>::Base> for #enum_ident {
            #[inline(always)]
            fn into_bits(self) -> ::modular_bitfield::private::Bits<<Self as ::modular_bitfield::Specifier>::Base> {
                ::modular_bitfield::private::Bits(
                    self as <Self as ::modular_bitfield::Specifier>::Base
                )
            }
        }
    ))
}
