extern crate proc_macro;
use proc_macro::TokenStream;
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{parse_macro_input, DeriveInput};
use syn::{Data, DataStruct, Fields};

/// Automatically implements the `Field` trait for each named field of a struct.
///
/// For a struct like:
///
/// ```rust
/// struct MyStruct {
///     foo: i32,
///     bar: String,
/// }
/// ```
///
/// This macro generates unit structs (e.g. `FooField`, `BarField`) and implements a
/// `Field` trait for each, enabling type-safe access and update of individual fields:
///
/// ```rust
/// pub struct FooField;
/// impl Field for FooField {
///     type FieldType = i32;
///     fn select<'a>(&self, store: &'a MyStruct) -> &'a Self::FieldType {
///         &store.foo
///     }
///     fn set(&self, store: &mut MyStruct, value: Self::FieldType) {
///         store.foo = value;
///     }
/// }
///
/// pub struct BarField;
/// impl Field for BarField {
///     type FieldType = String;
///     fn select<'a>(&self, store: &'a MyStruct) -> &'a Self::FieldType {
///         &store.bar
///     }
///     fn set(&self, store: &mut MyStruct, value: Self::FieldType) {
///         store.bar = value;
///     }
/// }
/// ```
///
/// This macro also generates the definition of the `Field` trait.
#[proc_macro_derive(FieldEnum)]
pub fn field_enum_derive(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;

    // Get the fields of the struct
    let fields = if let Data::Struct(DataStruct {
        fields: Fields::Named(ref fields_named),
        ..
    }) = input.data
    {
        &fields_named.named
    } else {
        panic!("FieldEnum can only be used with named fields");
    };

    // Generate unit structs and trait implementations
    let unit_structs = fields.iter().map(|field| {
        let ident = field.ident.as_ref().unwrap();
        let struct_name = format_ident!("{}Field", ident.to_string().to_case(Case::Pascal));
        let ty = &field.ty;
        quote! {
            pub struct #struct_name;

            impl Field for #struct_name {
                type FieldType = #ty;
                fn select<'a>(&self, store: &'a #name) -> &'a Self::FieldType {
                    &store.#ident
                }
                fn set(&self, store: &mut #name, value: Self::FieldType) {
                    store.#ident = value;
                }
            }
        }
    });

    // Generate the Field trait
    let expanded = quote! {
        pub trait Field {
            type FieldType;
            fn select<'a>(&self, store: &'a #name) -> &'a Self::FieldType;
            fn set(&self, store: &mut #name, value: Self::FieldType);
        }

        #(#unit_structs)*
    };

    TokenStream::from(expanded)
}

/// Generates code for atomic, thread-safe access to a struct via a global static store.
///
/// This macro produces:
///
/// - A static store (a `LazyLock` holding an `RwLock` protecting an `Arc` of the struct)
///   whose identifier is based on the struct name (e.g. `CRETE_FOO` for a struct named `Foo`).
///
/// - An implementation of several associated methods on the struct:
///     - `new()`: constructs a new instance using the struct's `Default` implementation.
///     - `read()`: returns an `Arc`-wrapped shared reference to the current state.
///     - `clone()`: (if the struct implements `Clone`) returns a cloned instance of the stored value.
///     - `write(self)`: atomically replaces the current stored value with the provided one.
///     - `select_ref<F: Field>(&self, field: F) -> &F::FieldType`: returns a reference to the selected field.
///     - `get<F, R>(field: F, f: impl FnOnce(&F::FieldType) -> R) -> R`: applies a closure to a shared reference
///       of the selected field.
///     - `select<F: Field>(field: F) -> F::FieldType`: returns a cloned value of the selected field.
///     - `set<F>(field: F, value: F::FieldType)`: updates a specific field and writes the new state to the store.
///     - `update(f: impl FnOnce(&mut Self))`: applies a mutation closure to the current state and updates the store.
///     - `update_async(f: impl AsyncFnOnce(&mut Self))`: an asynchronous version of `update` for non-blocking mutations.
///
/// The generated code leverages `std::sync::LazyLock`, `RwLock`, and `Arc` to ensure that all operations
/// are safe to use concurrently from multiple threads.
#[proc_macro_derive(Crete)]
pub fn all_together_derive(input: TokenStream) -> TokenStream {
    let input_clone = input.clone();

    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as DeriveInput);

    // Get the name of the struct
    let struct_name = &input.ident;

    // Generate the code from field_enum_derive
    let field_enum_tokens: TokenStream2 = field_enum_derive(input_clone).into();

    // Create a static identifier based on the struct name
    let crete_store_ident = format_ident!("CRETE_{}", struct_name.to_string().to_uppercase());

    // Check if the struct implements the `Clone` trait
    let implements_clone = input.attrs.iter().any(|attr| {
        if attr.path().is_ident("derive") {
            let mut found_clone = false;
            let _ = attr.parse_nested_meta(|nested| {
                if nested.path.is_ident("Clone") {
                    found_clone = true;
                }
                // Don't fail on non-`Clone` traits; keep parsing
                Ok(())
            });
            found_clone
        } else {
            false
        }
    });
    dbg!(&implements_clone);

    // Conditionally generate the `clone` function
    let clone_fn = if implements_clone {
        quote! {
            pub fn clone() -> #struct_name
            {
                let mut a = #crete_store_ident.read().expect("RWLock poisoned").clone();
                ::std::sync::Arc::<#struct_name>::make_mut(&mut a).clone()
            }
        }
    } else {
        quote! {} // Do not generate the function if Clone is not implemented
    };

    // Generate the necessary code
    let code = quote! {
        #field_enum_tokens

        static #crete_store_ident: ::std::sync::LazyLock<::std::sync::RwLock<::std::sync::Arc<#struct_name>>> =
            ::std::sync::LazyLock::new(|| ::std::sync::RwLock::new(::std::sync::Arc::new(#struct_name::new())));

        impl #struct_name {
            pub fn new() -> Self {
                #struct_name::default()
            }

            pub fn read() -> ::std::sync::Arc<#struct_name> {
                #crete_store_ident.read().expect("RWLock poisoned").clone()
            }

            #clone_fn

            pub fn write(self) {
                *#crete_store_ident.write().expect("RWLock poisoned") = ::std::sync::Arc::new(self);
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
                let mut s = ::std::sync::Arc::get_mut(&mut *store_write_guard).expect("You have an extra strong reference. Did you have a read() binding in the same scope?");

                field.set(&mut s, value);

                *store_write_guard = ::std::sync::Arc::new(::std::mem::take(s));
            }

            pub fn update(f: impl FnOnce(&mut #struct_name) -> ()) {
                let mut store_write_guard = #crete_store_ident.write().expect("RWLock poisoned");
                let mut s = ::std::sync::Arc::get_mut(&mut *store_write_guard).expect("You have an extra strong reference. Did you have a read() binding in the same scope?");


                f(&mut s);

                *store_write_guard = ::std::sync::Arc::new(::std::mem::take(s));
            }

            pub async fn update_async<F>(f: F)
            where
                F: AsyncFnOnce(&mut #struct_name),
            {
                let mut store_write_guard = #crete_store_ident.write().expect("RWLock poisoned");
                let mut s = ::std::sync::Arc::get_mut(&mut *store_write_guard).expect("You have an extra strong reference. Did you have a read() binding in the same scope?");

                f(&mut s).await;

                *store_write_guard = ::std::sync::Arc::new(::std::mem::take(s));
            }
        }
    };

    code.into()
}