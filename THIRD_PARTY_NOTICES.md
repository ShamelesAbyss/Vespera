# Vespera third-party notices

This notice accompanies internal builds while distribution review is in progress. The proprietary license applies to Vespera material only. Component licenses and copyright notices remain in effect.

## Embedded graphics and art

| Component | Notice |
| --- | --- |
| Bevy's embedded Fira Mono subset | Digitized data copyright 2012–2015, The Mozilla Foundation and Telefonica S.A. SIL Open Font License 1.1. The [full notice](licenses/FiraMono-LICENSE.txt) comes from the [revision used by Bevy 0.20.1](https://github.com/bevyengine/bevy/blob/a0577b5c6a7b208719dd761c5c6a7a539e472a97/assets/fonts/FiraMono-LICENSE). |
| KayKit Space Base Bits 1.0 by Kay Lousberg | CC0 1.0 Universal. See the [upstream notice](assets/world/space-base/LICENSE.txt). |
| Kenney Space Kit via the TGE asset repack | CC0 1.0 Universal. See the [art credits](assets/world/ATTRIBUTION.md) for pinned sources and Vespera's changes. |

## Windows x64 dependency inventory

The [resolved Cargo inventory](licenses/windows-x64-cargo-inventory.txt) records 425 third-party package and version entries for the Windows x64 CLI and graphics build, including build dependencies. Its expressions are the package declarations as published. An alternative expression does not record a license election. A build dependency appearing here does not establish that its code ships in the executable. [Inventory command](https://github.com/ShamelesAbyss/Vespera/blob/main/docs/DEPENDENCIES.md)

Artifact review must reconcile these declarations with the source and files actually shipped. In particular, inspect the Ring package's bundled native code and its separate BoringSSL and other-bits notices, the bundled SQLite source, Unicode and ICU data and notices, trust-root data, platform runtime files, and the WiX bootstrapper payload. Use exact versions and preserve the complete upstream copyright, license, and notice text where required. Review both the MSI and Setup executable in addition to the portable archive.

Before distribution, record the applicable license choice for alternatives, identify which obligations apply jointly, and resolve any required attribution, modification notices, source or relinking offers, and toolchain redistribution terms. Confirm the provenance and distribution rights for Vespera branding and any contributed original material. Provider daemons and model weights are obtained separately and their terms remain the operator's responsibility.
