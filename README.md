# Lightweight asynchronous interface composition.

***This project is in a perpetually experimental state. You probably don't want to use this in production.***

`microacto-rs` is a tiny (~6kb source) library designed to help provide consistent, coherent interfaces for highly concurrent applications. 
It solves a different set of problems than conventional actor frameworks. It requires state to be thread-safe, with non-blocking access where appropriate.

It places minimal requirements on the user while providing scalable, fault-tolerant interfaces. Much of this is thanks
to the excellent `tokio` runtime and Rust's addition of async/await features. 

If you are not already using `tokio`, or you are not working in a primarily asynchronous workspace,
then this is probably not what you're looking for. It does not attempt to eliminate any of the boilerplate associated with combining asynchronous
and synchronous workflows.

While `microacto-rs` is suitable for many asynchronous projects, there are a few areas where it can be especially useful:
- Server + client microservices
- Lightweight interface composition across arbitrary threading models.
- Simple asynchronous message handling for more complex systems.

<sub>*[Fork this project!](https://github.com/Justin42/microacto-rs/fork) This project is tiny, and easy to understand and modify.*</sub>


---

## Don't use `microacto-rs` in your project

Don't add this to your `Cargo.toml`:

```toml
[dependencies.microacto-rs]
git = "https://github.com/Justin42/microacto-rs"
branch = "master"
```
Commits may frequenly break your interface. There is no stable branch. You can pin a revision using the commit hash.  
Pinning a revision is *required* if you want to make any guarantees of stability to consumers of your interface.

```toml
[dependencies.microacto-rs]
git = "https://github.com/Justin42/microacto-rs"
rev = "b34n542"
```

---

## Example

```rust
// TODO
```

---

#### License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>