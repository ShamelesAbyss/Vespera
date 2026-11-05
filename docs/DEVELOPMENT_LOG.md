# Development log

## 2026-11-04 — Milestone 0: inspect and plan

Read the supplied master directive before making project changes. Inspected accessible source locations and found no existing VESPERA project. The assessment is limited to this sandbox and does not cover the user's unconnected computer or repositories. Initially observed six CPU threads, approximately 7.8 GiB RAM, and no exposed GPU devices. Rust was not installed.

Installed official stable Rust/Cargo 1.99.1 with rustfmt and Clippy. Created a new `/home/ubuntu/vespera` directory and preserved the directive. Recorded the repository assessment, current and future architecture direction, initial dependency graph, workspace structure, hardware considerations, and measurable implementation criteria in `INITIAL_PLAN.md` before writing implementation code.

Selected three functional crates rather than creating empty crates for all future milestones. Reserved licensing pending the owner's decision and disabled package publication. No remote repository was connected or changed.

## 2026-11-04 — Milestone 1: implement the foundation

Implemented side-effect-free Rust domain contracts in `vespera-core`, strict TOML and reference validation in `vespera-config`, and a native CLI in `vespera-app`. Agent identity and memory addressing remain independent of model selection. Unknown model estimates remain optional values. Checked budget arithmetic is deliberately not a resource scheduler or hardware controller.

The core crate's first locked build could not run before a lockfile existed. A normal Cargo build resolved dependencies and generated the shared `Cargo.lock`; subsequent checks used `--locked`. The core's own build, 28 tests, formatting, strict Clippy, and documentation checks passed.

Implemented size-bounded configuration loading, unknown-field rejection, duplicate-ID and referential checks, registered capability coverage, conservative resource defaults, and a loopback-first URL policy. Config errors suppress raw TOML and endpoints. Embedded URL credentials are denied, and environment references are not resolved in this milestone. Added end-to-end CLI tests and read-only diagnostic parsing tests.

## 2026-11-04 — Integration, correction, and documentation

The first full-workspace run compiled and passed all tests, but strict Clippy rejected three nested conditionals in GPU-device-path inspection. Corrected the conditionals with Rust let chains without changing behavior. The first run's evidence remains in `artifacts/validation-initial.log`; it is not treated as an all-green validation.

The corrected final run passed formatting, all-target compilation, all 57 tests, strict Clippy, rustdoc with warnings denied, and the optimized release build. The release executable validated `config/local.toml` and produced truthful `doctor --json` output. The final log ends with `Validation complete: all commands passed`.

The sandbox's RAM increased during execution: the final diagnostic recorded 25,604,427,776 host RAM bytes, six available CPU threads, unknown root-cgroup limits, and no exposed GPU device paths. Initial and final memory observations are time-specific, not a fixed user-hardware claim. Neither report establishes effective inference capacity or measured VRAM.

Completed architecture, all milestone acceptance criteria, provider, agent, memory, draft external-protocol, dependency, and implementation documentation. Provider/API/scheduler/storage/simulation/3D behavior is explicitly future work. Bionic integration is gated on identifying the exact product/version/API rather than assuming compatibility.

Initialized a fresh local Git repository on branch `main` after validation. No commit, remote push, destructive Git operation, GitHub publication, service deployment, or model inference was performed. The supplied GitHub Actions file has not been executed on GitHub. Source packaging excludes `.git` and `target`; the lockfile, sample configuration, documentation, and validation artifacts are included.

## Next work

Proceed to Milestone 2: provider abstraction and one working Ollama adapter. First establish bounded streaming, timeout, cancellation semantics, explicit unsupported capabilities, safe transport policy, and offline protocol fixtures. Do not add 3D or complex autonomous execution before those contracts support them.
