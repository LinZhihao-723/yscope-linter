//! Reading what `syn` types name, as written in the source.

/// # Returns
///
/// `ty` without parentheses and invisible groups.
pub fn peel_type_parens(mut ty: &syn::Type) -> &syn::Type {
    loop {
        ty = match ty {
            syn::Type::Paren(type_paren) => &type_paren.elem,
            syn::Type::Group(type_group) => &type_group.elem,
            _ => return ty,
        };
    }
}

/// # Returns
///
/// The last segment of `ty` if it is a path, or [`None`] otherwise.
pub fn type_last_segment(ty: &syn::Type) -> Option<String> {
    match peel_type_parens(ty) {
        syn::Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

/// # Returns
///
/// The `Output` type of the first `Future<Output = ..>` bound of `ty` if `ty` is an `impl Trait`
/// type, or [`None`] otherwise.
pub fn impl_future_output(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::ImplTrait(type_impl_trait) = peel_type_parens(ty) else {
        return None;
    };
    type_impl_trait.bounds.iter().find_map(|bound| {
        let syn::TypeParamBound::Trait(trait_bound) = bound else {
            return None;
        };
        let last_segment = trait_bound.path.segments.last()?;
        if last_segment.ident != "Future" {
            return None;
        }
        let syn::PathArguments::AngleBracketed(arguments) = &last_segment.arguments else {
            return None;
        };
        arguments.args.iter().find_map(|argument| match argument {
            syn::GenericArgument::AssocType(assoc_type) if assoc_type.ident == "Output" => {
                Some(&assoc_type.ty)
            }
            _ => None,
        })
    })
}

/// # Returns
///
/// Whether `path`'s last segment ends with `Result`.
pub fn is_result_path(path: &syn::Path) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident.to_string().ends_with("Result"))
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;

    use super::*;

    fn future_output_of(text: &str) -> Option<String> {
        let ty: syn::Type = syn::parse_str(text).expect("valid type");
        impl_future_output(&ty).map(|output| output.to_token_stream().to_string())
    }

    #[test]
    fn impl_future_outputs_are_found_in_any_bound() {
        assert_eq!(
            future_output_of("impl Future<Output = (A, B)> + Send"),
            Some("(A , B)".to_owned())
        );
        assert_eq!(
            future_output_of("(impl Send + std::future::Future<Output = u8>)"),
            Some("u8".to_owned())
        );
        assert_eq!(future_output_of("impl Iterator<Item = (A, B)>"), None);
        assert_eq!(future_output_of("Pin<Box<dyn Future<Output = u8>>>"), None);
        assert_eq!(future_output_of("u8"), None);
    }

    #[test]
    fn result_paths_end_with_result() {
        for (text, expected) in [
            ("Result", true),
            ("anyhow::Result", true),
            ("DbResult", true),
            ("Option", false),
            ("ResultSet", false),
        ] {
            let path: syn::Path = syn::parse_str(text).expect("valid path");
            assert_eq!(is_result_path(&path), expected, "{text}");
        }
    }
}
