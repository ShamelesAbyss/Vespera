# Dependency decisions

The initial Milestone 1 foundation used seven direct third-party libraries shared through the root Cargo workspace. The current runtime also uses Tokio, Reqwest, Futures and Bytes. `Cargo.lock` records the current exact dependency graph; historical metadata/license artifacts may describe the initial foundation and must be regenerated for a release audit.

## Introduced dependencies

**Serde 1.0.229** provides typed serialization and deserialization, with its `derive` feature enabled. Its design supports using the same domain types with multiple data formats [1]. This fits a model-independent core and strict input schemas. The declared license is `MIT OR Apache-2.0`.

**serde_json 1.0.154** is used for JSON event round-trip tests and CLI diagnostic output. It is not used to treat arbitrary unvalidated model output as trusted state. The declared license is `MIT OR Apache-2.0`.

**uuid 1.29.0** supplies independent UUID identities with only the additional `v4` and `serde` features requested. Domain wrappers reject nil UUIDs and enforce their serialized string boundary. Randomness comes from the library's platform-backed implementation; IDs are not authentication secrets. The declared license is `Apache-2.0 OR MIT`.

**thiserror 2.0.21** creates typed domain, configuration, and application errors. Endpoint strings and TOML error excerpts are intentionally not retained in public configuration errors. The declared license is `MIT OR Apache-2.0`.

**toml 0.9.12+spec-1.1.0** parses operator-owned, human-readable configuration using Serde [2]. The selected 0.9 release line is bounded in the manifest and locked to the tested version; an upgrade is an explicit maintenance action, not an automatic dependency on whatever release is latest. Strict unknown-field rejection and a one-MiB input limit protect this boundary. The declared license is `MIT OR Apache-2.0`.

**url 2.5.8** parses endpoints into scheme, host, user information, port, query, and fragment. Its URL Standard parsing avoids home-grown string heuristics [3]. Validation admits only loopback endpoints by default and rejects embedded credentials. Its IDNA support brings ICU-related transitive dependencies; this cost is accepted for a correct URL boundary rather than an ad hoc parser. URL parsing does not enforce DNS or redirect policy at runtime. The declared license is `MIT OR Apache-2.0`.

**Clap 4.6.7** supplies argument validation, help, version reporting, and native subcommands using its `derive` feature [4]. It prevents unsupported runtime operations from being advertised as working commands. The declared license is `MIT OR Apache-2.0`.

These libraries were selected for narrow responsibilities, documentation, portable Rust APIs, and compatible permissive declared licenses. Compilation, strict Clippy, tests, and documentation generation validate architectural fit on the recorded toolchain. This is not a complete vulnerability, provenance, legal, or long-term maintenance audit. Review advisories, upstream maintenance, dependency licenses, and release impact before publication or deployment. Lockfile entries for other target platforms do not imply a JavaScript or Python runtime in the native application.

## Runtime dependencies and test features

Tokio supplies asynchronous provider I/O, coordination, timeouts and bounded admission. Reqwest supplies HTTP with Rustls TLS and redirect prohibition. Futures core/util and Bytes support provider streaming. These are implemented dependencies in the workspace manifest and lockfile; their presence does not establish complete protocol or transport-policy verification. Registry/agent dev dependencies enable Tokio `test-util` for deterministic virtual-clock expiry/deadline tests.

The CLI enables Tokio `signal` to route Ctrl+C through owned cancellation rather than abruptly abandoning local execution state. This adds the locked transitive entries `signal-hook-registry 1.4.8` and `errno 0.3.14`; no unrelated package versions were upgraded. Interactive signal delivery still needs manual cross-OS verification. The CLI uses a current-thread Tokio runtime; network concurrency does not require one worker thread per host CPU. Existing provider/registry/scheduler/agent workspace crates are now composed by the application library and executable.

## Deferred dependencies

The loopback worker service uses Hyper HTTP/1, Hyper-util's Tokio adapters and HTTP Body Util's streaming size limit. Authentication uses Ring HMAC-SHA256 and Subtle constant-time equality. The SDK uses Reqwest with default features disabled for HTTP. The broker and SDK both use Tokio Tungstenite 0.30 for WebSocket handshakes and framing. The lockfile adds its dependency subtree without changing existing package versions. SHA-1 is used for the WebSocket handshake accept value. Request authentication still uses HMAC-SHA256.

SQLite storage now uses exactly `rusqlite 0.41.1` with default features disabled and explicit `libsqlite3-sys`/`bundled` features. The native source bundle reports SQLite 3.54.1; FTS5 is exercised by actual retrieval tests. No separate database server, SQLCipher, loadable extensions, generated bindings or Wasm SQLite backend is enabled. The build requires a C compiler in addition to Rust. Optional vectors remain deferred until their provenance, rebuild/resource limits and retrieval quality can be verified.

Bevy and wgpu are deferred until the required native 3D visualization has a stable event subscription contract. They must not become dependencies of core orchestration or headless operation. gRPC remains deferred.

No Python runtime, Python orchestration backend, JS frontend, bundled model weights, vendor SDK, or GPU runtime is required by the current first-party application. Node.js was used only as an initial development-environment utility to format Cargo metadata; it is not part of the repository's build or application requirements. Explicit CLI operations now call real provider adapters; a compatible daemon/model must be independently installed/configured for live inference.

## Toolchain and reproducibility

The installed and tested toolchain is Rust/Cargo 1.99.1. `rust-toolchain.toml` and package metadata pin that release; older Rust versions were not tested. Workspace resolver 2, shared dependency declarations, a committed lockfile, and `--locked` validation follow Cargo's workspace facilities [5]. These reproduce resolution, not byte-identical artifacts across different operating systems or compilers.

Unsafe Rust is forbidden for workspace code. This lint does not claim that third-party crates contain no unsafe implementation. Release builds currently use Cargo's default optimized profile; no thin-LTO or symbol-stripping profile is configured. Performance benchmarks and hardware-fit profiling remain later milestones.

## References

[1]: https://serde.rs/ "Official Serde serialization framework documentation"
[2]: https://docs.rs/toml "Rust TOML crate documentation"
[3]: https://docs.rs/url/latest/url/ "Rust URL parser documentation"
[4]: https://docs.rs/clap/latest/clap/ "Clap native command line parser documentation"
[5]: https://doc.rust-lang.org/cargo/reference/workspaces.html "Cargo workspace dependency and lockfile reference"

The bounded orchestrator adds one first-party crate using existing locked Serde/JSON, Tokio and Thiserror dependencies. No new third-party package or version is introduced. Its dependency direction is orchestrator to agents/providers/core; there is no reverse dependency or graphical runtime.

The memory addition locks six new third-party packages: `rusqlite 0.41.1`, `libsqlite3-sys 0.39.1`, `fallible-iterator 0.3.0`, `fallible-streaming-iterator 0.1.9`, `pkg-config 0.3.34` and `vcpkg 0.2.15`. Existing `cc`, `smallvec` and `bitflags` are reused. No existing package version or unrelated lockfile edge is changed. Upstream references: [Rusqlite](https://github.com/rusqlite/rusqlite), [SQLite FTS5](https://www.sqlite.org/fts5.html) and [application/schema pragmas](https://www.sqlite.org/pragma.html). Third-party native SQLite code is a dependency; first-party implementation remains Rust. This is not a full dependency security/license audit.
