# Vendored source

This directory is a source snapshot of the `clark-capture` Rust SDK at commit
`05941cb`, with the same subsequent clippy cleanup mirrored in that standalone
checkout, plus only the local Cargo package metadata required to build outside
its workspace.

Clark Desktop vendors the SDK because no Git or crates.io distribution exists
yet. Replace this snapshot with a pinned remote dependency once the standalone
repository is published; do not fork its event schema or client behavior here.
