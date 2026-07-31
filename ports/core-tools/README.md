# JustTools core engines

This internal workspace crate contains the pure-Rust implementations used by
the root `just` multicall executable:

- JSON formatting, validation, querying, sorting, and minification
- PDF inspection, merge, split, extraction, and rotation
- QR generation as PNG, SVG, or compact terminal output
- conservative SVGOMG-style optimization through OXVG
- native socket/process inspection for `justport`

The tiny `src/bin/just*.rs` targets exist only to give these engines direct CLI
integration coverage. Release packaging builds `-p justtools`, so users receive
one root executable and `just install` creates all command aliases.

The crate has no runtime dependency on Node, PowerShell, or external codecs.
Its focused test suite runs with:

```sh
cargo test --locked -p justtools-core --all-targets
```

OXVG is Rust-native and SVGO/SVGOMG-inspired, but not byte-identical to SVGO.
The conservative preset preserves IDs, `viewBox`, titles, descriptions, XML
namespaces, and accessibility attributes. PDF page operations preserve page
order and inheritable page resources; outlines and page labels are omitted when
they cannot be remapped safely. Socket owner visibility varies with OS support
and caller permissions, and unknown owners are never killable.
