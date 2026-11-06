# Vendored Switchyard

Source: https://github.com/NVIDIA-NeMo/Switchyard at commit
`c365efe832822541024704aadf6a0d5c81cc0528` (crate version 0.3.0).

Only `crates/protocol` and `crates/switchyard-translation` are vendored, with
`Cargo.toml` rewritten from workspace inheritance to standalone manifests and
`rust-version` lowered from 1.96.1 to 1.96.0 for the project toolchain.

Upstream license: Apache-2.0 (see `LICENSE` and `NOTICE` in each directory).
