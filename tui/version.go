package main

// coreVersion is the bundled core version, injected by the release scripts via
// -ldflags "-X main.coreVersion=...". It defaults to "dev" for local builds and
// is used to key the cache path of the embedded core in the macOS standalone
// build (see embed_core.go).
var coreVersion = "dev"
