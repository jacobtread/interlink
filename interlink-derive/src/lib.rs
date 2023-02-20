use darling::{FromDeriveInput, FromMeta};
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Ident, Type};

#[derive(FromDeriveInput, Default)]
#[darling(default, attributes(msg), forward_attrs(allow, doc, cfg))]
struct MessageOpts {
    rtype: Option<Type>,
}

/// Macro for deriving the Message trait to allow something to be
/// sent as a message
///
///
/// ```
/// use interlink::prelude::*;
///
/// /// Default message response type is ()
/// #[derive(Message)]
/// struct MyMessage;
///
/// /// Message with a string response type
/// #[derive(Message)]
/// #[msg(rtype = "String")]
/// struct MySecondMessage;
///
/// ```
#[proc_macro_derive(Message, attributes(msg))]
pub fn derive_message_impl(input: TokenStream) -> TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    let opts: MessageOpts = MessageOpts::from_derive_input(&input).expect("Invalid options");

    let ident: Ident = input.ident;

    let rtype: Type = match opts.rtype {
        // Custom response type
        Some(value) => value,
        // Default unit response type
        None => Type::from_string("()").unwrap(),
    };

    quote! {
        impl interlink::msg::Message for #ident {
            type Response = #rtype;
        }
    }
    .into()
}

/// Macro for deriving a basic Service implementation without
/// having any of the started and stopping hooks
#[proc_macro_derive(Service)]
pub fn derive_service_impl(input: TokenStream) -> TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    let ident = input.ident;
    quote! {
        impl interlink::service::Service for #ident { }
    }
    .into()
}
