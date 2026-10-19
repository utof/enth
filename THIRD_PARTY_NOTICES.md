# Third-party notices

enth is MIT-licensed. This file lists source code **ported or copied** into enth from other projects, with the notices their licences require. Crate dependencies are not listed here: `cargo deny` checks their licences against `deny.toml`, and their list is generated from `Cargo.lock`.

Rules: [docs/adr/0004-licensing.md](docs/adr/0004-licensing.md) (which licences ported source may carry) and [docs/adr/0006-effects-strategy.md](docs/adr/0006-effects-strategy.md). Each port also has a provenance note in `docs/dsp/<module>.md`.

## Entry format

```markdown
### <Upstream name>

- Upstream: <URL> at commit `<full hash>`
- Licence: <SPDX id, or LicenseRef-STK-4.3>
- Copyright: <upstream copyright line, verbatim>
- Derived enth files: `crates/…`
- Changes from upstream: <e.g. RNG replaced by the engine's seeded PRNG; denormals flushed in code>

<full licence text, verbatim>
```

## Entries

None yet: no third-party source has been ported.

Expected in P1, each added in the commit that lands its port:

- Signalsmith Basics (`reverb.h` and helpers) and the Signalsmith DSP library — MIT
- Julius O. Smith's Faust `compressor_mono` — STK-4.3
- web-audio-api-rs `dynamics_compressor.rs` — MIT, only if code (not just the cross-check) is ported

Later: CloudSeedCore — MIT, with credit to Ghost Note Audio.
