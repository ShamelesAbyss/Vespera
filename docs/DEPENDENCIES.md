# Dependency decisions

Milestone 1 uses seven direct third-party libraries shared through the root Cargo workspace. `Cargo.lock` records the exact dependency graph. Cargo metadata and the complete declared license inventory are retained in `artifacts/cargo-metadata.json` and `artifacts/dependency-licenses.tsv`. Version and license statements below were obtained from that resolved metadata, not guessed from search results.

## Introduced dependencies

**Serde 1.0.229** provides typed serialization and deserialization, with its `derive` feature enabled. Its design supports using the same domain types with multiple data formats [1]. This fits a model-independent core and strict input schemas. The declared license is `MIT OR Apache-2.0`.

**serde_json 1.0.154** is used for JSON event round-trip tests and CLI diagnostic output. It is not used to treat arbitrary unvalidated model output as trusted state. The declared license is `MIT OR Apache-2.0`.

**uuid 1.29.0** supplies independent UUID identities with only the additional `v4` and `serde` features requested. Domain wrappers reject nil UUIDs and enforce their serialized string boundary. Randomness comes from the library's platform-backed implementation; IDs are not authentication secrets. The declared license is `Apache-2.0 OR MIT`.

**thiserror 2.0.21** creates typed domain, configuration, and application errors. Endpoint strings and TOML error excerpts are intentionally not retained in public configuration errors. The declared license is `MIT OR Apache-2.0`.

**toml 0.9.12+spec-1.1.0** parses operator-owned, human-readable configuration using Serde [2]. The selected 0.9 release line is bounded in the manifest and locked to the tested version; an upgrade is an explicit maintenance action, not an automatic dependency on whatever release is latest. Strict unknown-field rejection and a one-MiB input limit protect this boundary. The declared license is `MIT OR Apache-2.0`.

**url 2.5.8** parses endpoints into scheme, host, user information, port, query, and fragment. Its URL Standard parsing avoids home-grown string heuristics [3]. Validation admits only loopback endpoints by default and rejects embedded credentials. Its IDNA support brings ICU-related transitive dependencies; this cost is accepted for a correct URL boundary rather than an ad hoc parser. URL parsing does not enforce DNS or redirect policy at runtime. The declared license is `MIT OR Apache-2.0`.

**Clap 4.6.7** supplies argument validation, help, version reporting, and native subcommands using its `derive` feature [4]. It prevents unsupported runtime operations from being advertised as working commands. The declared license is `MIT OR Apache-2.0`.

These libraries were selected for narrow responsibilities, documentation, portable Rust APIs, and compatible permissive declared licenses. Compilation, strict Clippy, tests, and documentation generation validate architectural fit on the recorded toolchain. This is not a complete vulnerability, provenance, legal, or long-term maintenance audit. Review advisories, upstream maintenance, dependency licenses, and release impact before publication or deployment. Lockfile entries for other target platforms do not imply a JavaScript or Python runtime in the native application.

## Deferred dependencies

Tokio is a future asynchronous control-plane candidate, not needed for deterministic types or read-only CLI commands. Axum and WebSockets belong at a later authenticated API boundary. Reqwest or another Rust HTTP transport should be evaluated in Milestone 2 against streaming cancellation, timeout, redirect, TLS, and endpoint-policy requirements.

SQLite, through a chosen Rust database layer, is deferred until memory transactions, schema migration, namespaces, provenance, and recovery can be tested. Start with structured storage and text search; introduce vector retrieval only when embedding dimensions, model versions, indexing, resource limits, and retrieval quality are specified.

Bevy and wgpu are deferred until the optional native visualization has a stable event subscription contract. They must not become dependencies of core orchestration or headless operation. gRPC is deferred: REST plus WebSockets is the initial draft interoperability direction, not a deployed server.

No Python runtime, Python orchestration backend, JS frontend, local model weights, provider SDK, GPU runtime, or LLM API call was introduced. Node.js was used only as a development-environment utility to format Cargo metadata; it is not part of the repository's build or application requirements.

## Toolchain and reproducibility

The installed and tested stable toolchain is Rust/Cargo 1.99.1. `rust-toolchain.toml` pins that exact release. Package metadata requests Rust 1.99; older Rust versions were not tested. Workspace resolver 3, shared dependency declarations, a committed-ready lockfile, and `--locked` validation follow Cargo's workspace facilities [5]. These reproduce resolution, not byte-identical artifacts across different operating systems or compilers.

Unsafe Rust is forbidden for workspace code. This lint does not claim that third-party crates contain no unsafe implementation. Release builds use thin LTO and symbol stripping. Performance benchmarks and hardware-fit profiling remain later milestones.

## References

[1]: https://serde.rs/ "Official Serde serialization framework documentation"
[2]: https://docs.rs/toml "Rust TOML crate documentation"
[3]: https://docs.rs/url/latest/url/ "Rust URL parser documentation"
[4]: https://docs.rs/clap/latest/clap/ "Clap native command line parser documentation"
[5]: https://doc.rust-lang.org/cargo/reference/workspaces.html "Cargo workspace dependency and lockfile reference"
