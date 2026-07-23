# Third-Party Notices

Catalyst Code (`catcode`) is distributed under the MIT License (see
[`LICENSE`](LICENSE)). This file lists the third-party software that is
distributed with, or linked into, CatCode and that requires attribution.

## Microsandbox

When sandboxing is enabled, CatCode uses the official **Microsandbox** Rust SDK
(the `microsandbox` crate, pinned at version 0.6.6) to run agent-controlled
workloads inside a microVM. The SDK is linked into the CatCode core binary.

- **Project:** https://github.com/nicholasgasior/microsandbox (or the canonical
  `microsandbox` crate on crates.io)
- **License:** Apache License 2.0
- **Used for:** booting and managing lightweight microVMs that isolate agent
  process execution (bash, git, diagnostics, plugin scripts, …) on Linux via
  KVM, on Apple Silicon macOS, and on Windows via the Windows Hypervisor
  Platform.

### Apache License 2.0

```
Copyright 2024 Microsandbox contributors.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

Microsandbox in turn bundles third-party components (notably the `libkrunfw`
kernel bundle and `msb` runtime), which are downloaded on first use into a
CatCode cache directory. Those components retain their own upstream licenses;
see their respective repositories for details. CatCode does not modify them.

## Default sandbox image

The default sandbox image (`ghcr.io/catalystctl/catcode-sandbox`) bundles a
Debian base plus the Rust (rustup), Node.js (NodeSource), Python, and Go
toolchains. Each of those retains its upstream license:

- **Debian** — packages under their individual licenses (see
  `/usr/share/doc/*/copyright` inside the image).
- **Rust / rustup** — Apache-2.0 / MIT.
- **Node.js** — the Node.js license (MIT-compatible).
- **Python** — the PSF License.
- **Go** — the BSD 3-Clause License.

The image build definition lives at [`sandbox/Dockerfile`](sandbox/Dockerfile).
Each CatCode release records the resolved image digest it was built against in
its release notes.

## Other dependencies

CatCode links many other open-source Rust, Go, and TypeScript crates/packages.
Each is governed by its own license as declared on its registry
(crates.io / pkg.go.dev / npm). CatCode's use of these dependencies is
compatible with their respective licenses.
