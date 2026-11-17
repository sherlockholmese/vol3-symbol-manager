# Bundled dwarf2json

Source: https://github.com/volatilityfoundation/dwarf2json

Revision: `9f14607e0d339d463ea725fbd5c08aa7b7d40f75` (tool version 0.9.0).

`main.go`, `go.mod`, `go.sum`, and `LICENSE.txt` are unmodified upstream files.
The `vendor` directory was produced by `go mod vendor`; it includes pflag v1.0.5
and its BSD license. No network access is needed to build this converter.

`build.rs` compiles this source for Cargo's target with CGO disabled, and the
Rust binary embeds the result. At runtime it is written into the private
generation workspace and deleted with that workspace. `vol3sm licenses` prints
the bundled software's notices and licenses.

To update, replace the upstream files from an explicit revision, run
`go mod vendor` in this directory, and update this revision and the runtime
notices in `src/bundled.rs`. Preserve all copyright notices and licenses.
