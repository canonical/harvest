# Supported languages

Harvest parses these languages with tree-sitter and records their symbols (functions, methods, classes, types), calls, imports and inheritance:

| | | | |
|---|---|---|---|
| Ada | Bash | C | C# |
| C++ | CUDA | D | Dart |
| Elixir | Elm | Erlang | Fortran |
| F# | Gleam | GLSL | Go |
| Groovy | Haskell | Java | JavaScript |
| Julia | Kotlin | Lua | MATLAB |
| Nix | Objective-C | OCaml | Pascal |
| Perl | PHP | PowerShell | Protocol Buffers |
| Python | R | Racket | Ruby |
| Rust | Scala | Scheme | Solidity |
| Swift | TypeScript (and TSX) | Verilog | Zig |

Other text files (Markdown, YAML, configuration, templates) are indexed as text: they are searchable and readable but have no symbols or call graph.

The Workspace tool `search_workspace` searches any file in a Code session, whatever its language.
