use darling::FromAttributes;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{quote, ToTokens};
use syn::{FnArg, Ident};

fn parse_method_arg(arg: &FnArg) -> (Ident, Ident, bool) {
    if let syn::FnArg::Typed(a) = arg {
        match a.ty.as_ref() {
            syn::Type::Path(p) => {
                if let syn::Pat::Ident(i) = a.pat.as_ref() {
                    let attr_name = i.ident.clone();
                    let attr_type = p.path.segments[0].ident.clone();
                    return (attr_name, attr_type, false);
                }
            }
            syn::Type::Reference(p) => {
                if let syn::Pat::Ident(i) = a.pat.as_ref() {
                    if let syn::Type::Path(p) = p.elem.as_ref() {
                        let attr_name = i.ident.clone();
                        let attr_type = p.path.segments[0].ident.clone();
                        return (attr_name, attr_type, true);
                    }
                }
            }
            _ => {}
        };
    }
    panic!("unsupported function argument");
}

#[derive(Debug, FromAttributes)]
#[darling(attributes(rpc))]
struct MethodAttrs {
    #[darling()]
    name: Option<String>,
    #[darling()]
    result_field: Option<String>,
}

#[allow(clippy::too_many_lines)]
#[proc_macro_attribute]
/// # Panics
///
/// Will panic on invalid or unsupported
pub fn rpc_client(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item: syn::Item = syn::parse(input).expect("invalid input");
    if let syn::Item::Trait(trait_item) = item {
        let struct_name = Ident::new(&format!("{}Client", trait_item.ident), Span::call_site());
        let name = trait_item.ident;
        let mut methods = Vec::new();
        for item in trait_item.items {
            if let syn::TraitItem::Method(method) = item {
                assert!(
                    (method.sig.ident != "get_rpc_client"),
                    "get_rpc_client is a reserved name"
                );
                let attrs = match MethodAttrs::from_attributes(&method.attrs) {
                    Ok(v) => v,
                    Err(e) => return TokenStream::from(e.write_errors()),
                };
                let method_name = method.sig.ident.clone();
                let rpc_method_name = if let Some(name) = attrs.name {
                    Ident::new(&name, Span::call_site())
                } else {
                    method.sig.ident
                };
                let ty: Option<syn::Type> = match method.sig.output {
                    syn::ReturnType::Type(_, ty) => Some(*ty),
                    syn::ReturnType::Default => None,
                };
                let mut refs_found = false;
                let mut input_struct_names = Vec::new();
                let mut input_struct_args = Vec::new();
                let inputs = method.sig.inputs;
                let ret = if let Some(t) = ty {
                    if let syn::Type::Path(tpath) = t {
                        let r = tpath.path.segments[0].ident.clone();
                        quote! { #r }
                    } else {
                        panic!("unsupported return type");
                    }
                } else {
                    quote! { () }
                };
                for arg in inputs.iter().skip(1) {
                    let (name, tp, is_ref) = parse_method_arg(arg);
                    input_struct_names.push(quote! {
                        #name,
                    });
                    if is_ref {
                        refs_found = true;
                        input_struct_args.push(quote! {
                            #name: &'a #tp,
                        });
                    } else {
                        input_struct_args.push(quote! {
                            #name: #tp,
                        });
                    }
                }
                let (input_struct, payload) = if input_struct_args.is_empty() {
                    let p = quote! {
                        ()
                    };
                    (None, p)
                } else {
                    let lifetime = if refs_found {
                        Some(quote! { <'a> })
                    } else {
                        None
                    };
                    let s = Some(quote! {
                        #[derive(serde::Serialize)]
                        struct InputPayload #lifetime {
                            #(#input_struct_args)*
                        }
                    });
                    let p = quote! {
                        InputPayload {
                            #(#input_struct_names)*
                        }
                    };
                    (s, p)
                };
                let (response_tp, out, output_struct) =
                    if let Some(result_field) = attrs.result_field {
                        let field = Ident::new(&result_field, Span::call_site());
                        let output_type = Ident::new("OutputPayload", Span::call_site());
                        (
                            Some(quote! {
                                #output_type
                            }),
                            quote! {
                                Ok(response.#field)
                            },
                            Some(quote! {
                                #[derive(serde::Deserialize)]
                                struct OutputPayload {
                                    #field: #ret
                                }
                            }),
                        )
                    } else {
                        (
                            Some(ret.clone()),
                            quote! {
                                Ok(response)
                            },
                            None,
                        )
                    };
                let f = quote! {
                    fn #method_name(#inputs) -> Result<#ret, ::bma_jrpc::Error> {
                        #input_struct
                        #output_struct
                        let response: #response_tp = self.get_rpc_client().call(
                            stringify!(#rpc_method_name), #payload)?;
                        #out
                    }
                };
                methods.push(f);
            }
        }
        let f = quote! {
            trait #name<X: ::bma_jrpc::Rpc> {
                #(#methods)*
                fn get_rpc_client(&self) -> &X;
            }
            struct #struct_name<X: ::bma_jrpc::Rpc> {
                client: X
            }
            impl<X: ::bma_jrpc::Rpc> #struct_name<X> {
                fn new(client: X) -> Self {
                    Self { client }
                }
            }
            impl<X: ::bma_jrpc::Rpc> #name<X> for #struct_name<X> {
                fn get_rpc_client(&self) -> &X {
                    &self.client
                }
            }
        };
        f.into_token_stream().into()
    } else {
        panic!("the attribute must be placed on a trait");
    }
}
