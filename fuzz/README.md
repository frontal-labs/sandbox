# Fuzz

Fuzz testing targets for the sandbox system.

## Status

**Empty.** No fuzz targets have been implemented yet.

## Planned Targets

- Path normalization and resolution in `sandbox-vfs`
- Protobuf message parsing in `sandbox-proto`
- Policy evaluation in `sandbox-policy`
- CAS blob handling in `sandbox-cas`
- VMM/guest protocol handling in `sandbox-vmm` / `sandbox-guest`

## Implementation

Fuzz targets will use `cargo-fuzz` (libFuzzer) and be integrated into CI.

## License

Apache-2.0