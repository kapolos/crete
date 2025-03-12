# Crete

Crete is a procedural macro that simplifies state management in Rust, in a flexible way. 

It generates code for atomic access to a struct's fields using an `RwLock` and a static store, making it easier to work 
with shared state in both synchronous and asynchronous contexts.

Because users can implement practically anything on the struct, Crete allows for a flexible store that can be tailored 
to a variety of needs. 
Perhaps you'd like to use to build a Redux-style store.
Perhaps you enjoy having a billion custom setters.
Perhaps you just enjoy mutating everything directly.
Or maybe you even have your own homemade style of setters and getters that you love best?

No restrictions.

## Features

- **Ergonomic, intuitive Design**  
  Reduces boilerplate with a straightforward interface for managing state.

- **Synchronous and Asynchronous Support**  
  Works seamlessly in both synchronous and asynchronous environments.

- **Versatile Clone Support**  
  Adapts to your needs by supporting both cloneable and non-cloneable types.

- **No shoehorning**  
  Build the State management style you enjoy best on top... or not.

### Details

* Generates code for atomic, thread-safe access to a struct by defining a static store typically within a module 
that holds and manages the shared state.

* Generates unit structs (e.g. `FooField`, `BarField`) and implements a `Field` trait for each,
enabling type-safe access and update of individual fields.

 This macro produces:

 - A static store (a `LazyLock` holding an `RwLock` protecting an `Arc` of the struct)
   whose identifier is based on the struct name (e.g. `CRETE_FOO` for a struct named `Foo`).

 - An implementation of several associated methods on the struct:
     - `new()`: constructs a new instance using the struct's `Default` implementation. Called by `LazyLock`.
     - `read()`: returns an `Arc`-wrapped shared reference to the current state.
     - `clone()`: (if the struct implements `Clone`) returns a cloned instance of the stored value.
     - `write(self)`: atomically replaces the current stored value with the provided one.
     - `select_ref<F: Field>(&self, field: F) -> &F::FieldType`: returns a reference to the selected field.
     - `get<F, R>(field: F, f: impl FnOnce(&F::FieldType) -> R) -> R`: applies a closure to a shared reference
       of the selected field.
     - `select<F: Field>(field: F) -> F::FieldType`: returns a cloned value of the selected field.
     - `set<F>(field: F, value: F::FieldType)`: updates a specific field and writes the new state to the store.
     - `update(f: impl FnOnce(&mut Self))`: applies a mutation closure to the current state and updates the store.
     - `update_async(f: impl AsyncFnOnce(&mut Self))`: an asynchronous version of `update` for non-blocking mutations.

 The generated code leverages `std::sync::LazyLock`, `RwLock`, and `Arc` to ensure that all operations
 are safe to use concurrently from multiple threads.

## How it works


### Static Store

A static store is created for the struct, allowing atomic access to its state:

```rust
static #crete_store_ident: ::std::sync::LazyLock<::std::sync::RwLock<::std::sync::Arc<#struct_name>>> =
    ::std::sync::LazyLock::new(|| ::std::sync::RwLock::new(::std::sync::Arc::new(#struct_name::new())));
```

### With a struct like this:

```rust
use crete::Crete; 
 
#[derive(Crete, Default, Clone, Debug)] 
pub struct Store { 
 pub field1: String, 
 pub field_foo: String, 
 pub toggle: bool, 
 pub index: u32
}

impl Store {
    pub async fn inc1(&mut self) {
        self.index += 1;
    }

    pub fn dec2(&mut self) {
        self.index -= 2;
    }
}
```

### You can now do this:

```rust
#[test]
#[serial]
fn doc() {
    // Some initial values
    Store::set(Field1Field, "test value".to_string());
    Store::set(FieldFooField, "Foo".to_string());
    Store::set(ToggleField, true);
    Store::set(IndexField, 1000);

    /******************/
    /****** READ ******/
    /******************/
    
    // Get an owned value (only available if a field is Clone)
    let field1_value = Store::select(Field1Field);
    let field_foo_value = Store::select(FieldFooField);
    let toggle_value = Store::select(ToggleField);
    let index_value = Store::select(IndexField);
    assert_eq!(field1_value, "test value".to_string());
    assert_eq!(field_foo_value, "Foo".to_string());
    assert_eq!(toggle_value, true);
    assert_eq!(index_value, 1000);

    // Use the closure-based `get` method to get a reference
    Store::get(Field1Field, |value| {
        // `value` is a reference to the field
        assert_eq!(value, &"test value".to_string());
    });

    // You could also get a reference via binding since this is an RWLock<Arc<F>
    {
        let binding = Store::read();
        let field1_ref = binding.select_ref(Field1Field);
        assert_eq!(field1_ref, &"test value".to_string());
    }

    /******************/
    /****** WRITE *****/
    /******************/

    // Update via closure
    Store::update(|s| { // &mut Store
        s.field1 = "updated value".to_string();
        s.dec2();
    });
    assert_eq!(Store::select(Field1Field), "updated value".to_string());
    assert_eq!(Store::read().index, 998);

    // And we can use `set()` as we saw earlier
    Store::set(FieldFooField, "Foo".to_string());
    assert_eq!(Store::select(FieldFooField), "Foo".to_string());
}
```

### Async closure support

```rust
    #[tokio::test]
    async fn doc2() {
        Store::set(IndexField, 1000);

        // Async closure
        Store::update_async(async |s| { // &mut Store
            s.field1 = "updated value 2".to_string();
            s.inc1().await;
            s.inc1().await;
            s.inc1().await;
        }).await;

        assert_eq!(Store::select(IndexField), 1003);
        Store::get(Field1Field, |value| {
            assert_eq!(value, "updated value 2");
        });
    }
```

### No Clone, No Problem

It works the same way, except:
* `select()` does not exist for fields that are not clone-able.
* `clone()` does not exist for the static Struct.

```rust
#[cfg(test)]
mod tests_no_clone {
    use tokio;
    use crete::Crete;

    #[derive(Debug, PartialEq)]
    pub struct NotCloneType {
        pub value: i32,
    }

    impl Default for NotCloneType {
        fn default() -> Self {
            NotCloneType { value: 0 }
        }
    }

    #[derive(Crete, Default)]
    pub struct Store {
        pub foo: NotCloneType,
    }

    impl Store {
        async fn reset(&mut self) {
            self.foo = NotCloneType::default();
        }

        fn init(&mut self) {
            self.foo = NotCloneType { value: 42 };
        }
    }

    #[tokio::test]
    async fn foobar() {
        Store::set(FooField, NotCloneType { value: 100 });
        Store::get(FooField, |v| {
            assert_eq!(v, &NotCloneType { value: 100 });
        });

        Store::update_async(async |s| {
            s.init();
            assert_eq!(s.foo, NotCloneType { value: 42 });

            s.reset().await;
        }).await;
        Store::get(FooField, |v| {
            assert_eq!(v, &NotCloneType { value: 0 });
        });
    }
}
```

### Considerations

You are using `RWLock` behind the scenes. The usual considerations for locking in multithreaded code apply.

In essence:
* Don't let your thread panic while it holds a write [lock](https://doc.rust-lang.org/src/std/sync/poison/rwlock.rs.html#370-375).
* If the thread already has acquired a lock, [don't](https://doc.rust-lang.org/src/std/sync/poison/rwlock.rs.html#460-465) try to get it again before it drops.

## FAQ

### What's with the name?

* It's interestingly confusing with `crate`.
* It's the name of the island the crate author spends lots of his time at.

## License

This project is licensed under the MIT License._