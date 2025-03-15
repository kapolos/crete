extern crate proc_macro;
use proc_macro::TokenStream;
use convert_case::{Case, Casing};
use quote::{format_ident, quote};
use syn::{parse_macro_input, DeriveInput};
use syn::{Data, DataStruct, Fields};
use syn::parse::{Parse, ParseStream, Result};
use syn::{Ident, Token};
use syn::spanned::Spanned;

/// Parser for attribute arguments.
struct CreteArgs {
    clone: bool
}

impl Parse for CreteArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut clone = false;

        // If the input is empty, no attributes were provided.
        if input.is_empty() {
            return Ok(CreteArgs { clone: false });
        }

        // Parse comma-separated identifiers.
        while !input.is_empty() {
            let ident: Ident = input.parse()?;

            if ident == "Clone" {
                if clone {
                    // Error if "Clone" is specified more than once.
                    return Err(syn::Error::new(
                        ident.span(),
                        "Duplicate 'Clone' attribute",
                    ));
                }
                clone = true;
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unexpected attribute '{}'. Expected 'Clone'.", ident),
                ));
            }

            // If there's more input, expect a comma.
            if !input.is_empty() {
                let comma: Token![,] = input.parse()?;
                if input.is_empty() {
                    return Err(syn::Error::new(
                        comma.span(),
                        "Trailing comma not allowed",
                    ));
                }
            }
        }

        Ok(CreteArgs { clone })
    }
}

#[proc_macro_attribute]
pub fn crete(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse attribute parameters using our custom parser.
    let args = parse_macro_input!(attr as CreteArgs);

    // Parse the struct definition.
    let input = parse_macro_input!(item as DeriveInput);
    let struct_name = &input.ident;

    // Check if the struct derives Clone.
    let derives_clone = input.attrs.iter().any(|attr| {
        if attr.path().is_ident("derive") {
            let mut found_clone = false;
            let _ = attr.parse_nested_meta(|nested| {
                if nested.path.is_ident("Clone") {
                    found_clone = true;
                }
                Ok(())
            });
            found_clone
        } else {
            false
        }
    });

    // * If the user types #[crete()] BEFORE #[derive(Clone)], it is implied -> #[crete(Clone)].
    // * Allows the user to force true if the struct doesn't derive it but does impl manually.
    let struct_is_clone = args.clone || derives_clone;
    // println!("Struct name: {}, Struct is clone: {}, args.clone: {}, impl Clone: {}", struct_name, struct_is_clone, args.clone, derives_clone);

    // Extract the fields (only works with named fields).
    let fields = if let Data::Struct(DataStruct {
                                         fields: Fields::Named(ref fields_named),
                                         ..
                                     }) = input.data
    {
        &fields_named.named
    } else {
        panic!("Crete can only be used with named fields");
    };

    // Define the Field trait. Note the store parameter is of type `#struct_name` (e.g. Store)
    let field_trait = quote! {
        pub trait Field {
            type FieldType;
            fn select<'a>(&self, store: &'a #struct_name) -> &'a Self::FieldType;
            fn set(&self, store: &mut #struct_name, value: Self::FieldType);
        }
    };

    // Generate unit structs and their implementations for each field.
    let unit_structs = fields.iter().map(|field| {
        let ident = field.ident.as_ref().unwrap();
        let unit_struct_name = format_ident!("{}Field", ident.to_string().to_case(Case::Pascal));
        let ty = &field.ty;
        quote! {
            pub struct #unit_struct_name;

            impl Field for #unit_struct_name {
                type FieldType = #ty;
                fn select<'a>(&self, store: &'a #struct_name) -> &'a Self::FieldType {
                    &store.#ident
                }
                fn set(&self, store: &mut #struct_name, value: Self::FieldType) {
                    store.#ident = value;
                }
            }
        }
    });

    // Create the static store identifier.
    let crete_store_ident = format_ident!("CRETE_{}", struct_name.to_string().to_uppercase());

    let impl_block = if struct_is_clone {
        quote! {
            impl #struct_name {
                pub fn new() -> Self {
                    #struct_name::default()
                }

                pub fn read() -> Arc<#struct_name> {
                    #crete_store_ident.read().expect("RWLock poisoned").clone()
                }

                pub fn clone() -> #struct_name {
                    let mut a = #crete_store_ident.read().expect("RWLock poisoned").clone();
                    Arc::make_mut(&mut a).clone()
                }

                pub fn write(self) {
                    *#crete_store_ident.write().expect("RWLock poisoned") = Arc::new(self);
                }

                pub fn select_ref<F: Field>(&self, field: F) -> &F::FieldType {
                    field.select(self)
                }

                pub fn get<F, R>(field: F, f: impl FnOnce(&F::FieldType) -> R) -> R
                where
                    F: Field,
                {
                    let store = #struct_name::read();
                    let field_ref = store.select_ref(field);
                    f(field_ref)
                }

                pub fn select<F: Field>(field: F) -> F::FieldType
                where
                    F::FieldType: Clone,
                {
                    #struct_name::read().select_ref(field).clone()
                }

                pub fn set<F>(field: F, value: F::FieldType)
                where
                    F: Field,
                {
                    let mut store_write_guard = #crete_store_ident.write().expect("RWLock poisoned");
                    let mut s = Arc::make_mut(&mut *store_write_guard);

                    field.set(&mut s, value);
                }

                pub fn update(f: impl FnOnce(&mut #struct_name) -> ()) {
                    let mut store_write_guard = #crete_store_ident.write().expect("RWLock poisoned");
                    let s = Arc::make_mut(&mut *store_write_guard);

                    f(s);
                }

                pub async fn update_async<F>(f: F)
                where
                    F: AsyncFnOnce(&mut #struct_name),
                {
                    let mut store_write_guard = #crete_store_ident.write().expect("RWLock poisoned");
                    let s = Arc::make_mut(&mut *store_write_guard);

                    f(s).await;
                }
            }
        }
    } else {
        quote! {
            impl #struct_name {
                pub fn new() -> Self {
                    #struct_name::default()
                }

                pub fn write(self) {
                    let store_arc = #crete_store_ident.clone();
                    let mut store_guard = store_arc.write().expect("RWLock poisoned");
                    *store_guard = self;
                }

                pub fn select_ref<F: Field>(&self, field: F) -> &F::FieldType {
                    field.select(self)
                }

                pub fn get<F, R>(field: F, f: impl FnOnce(&F::FieldType) -> R) -> R
                where
                    F: Field,
                {
                    let store_arc = #crete_store_ident.clone();
                    let store_guard = store_arc.read().expect("RWLock poisoned");
                    let field_ref = field.select(&*store_guard);

                    f(field_ref)
                }

                pub fn set<F>(field: F, value: F::FieldType)
                where
                    F: Field,
                {
                    let store_arc = #crete_store_ident.clone();
                    let mut store_guard = store_arc.write().expect("RWLock poisoned");

                    field.set(&mut *store_guard, value);
                }

                pub fn update(f: impl FnOnce(&mut #struct_name) -> ()) {
                    let store_arc = #crete_store_ident.clone();
                    let mut store_guard = store_arc.write().expect("RWLock poisoned");

                    f(&mut *store_guard);
                }

                pub async fn update_async<F>(f: F)
                where
                    F: AsyncFnOnce(&mut #struct_name),
                {
                    let store_arc = #crete_store_ident.clone();
                    let mut store_guard = store_arc.write().expect("RWLock poisoned");

                    f(&mut *store_guard).await;
                }
            }
        }
    };

    let static_store = if struct_is_clone {
        quote! {
            static #crete_store_ident: LazyLock<RwLock<Arc<#struct_name>>> =
                LazyLock::new(|| RwLock::new(Arc::new(#struct_name::new())));
        }
    } else {
        quote! {
            static #crete_store_ident: LazyLock<Arc<RwLock<#struct_name>>> =
                LazyLock::new(|| Arc::new(RwLock::new(#struct_name::new())));
        }
    };

    let expanded = quote! {
        use std::sync::{Arc, RwLock, LazyLock};

        #input

        #field_trait

        #(#unit_structs)*

        #static_store

        #impl_block
    };

    TokenStream::from(expanded)
}
