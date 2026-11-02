# Veridoc

Forensic analysis for scanned and digital documents. Veridoc inspects
documents for visual tampering and cross-checks the information they
contain, tracing every finding back to its source region.

## Layout

| Path | Contents |
|------|----------|
| `crates/core` | Rust analysis engine |
| `web` | Web client |
| `android` | Android app |
| `docs` | Architecture and technical notes |
| `data/samples` | Synthetic sample documents |

## Requirements

- Rust 1.75 or newer

## Build and test

    cargo build --release
    cargo test --all

## License

MIT. See `LICENSE`.