// Test-only transport boundary; API methods, App and GatewayClient are unmodified.
// This does NOT verify Rust plugin permissions; run `test:acceptance:native` for that.
export const fetch = (...args) => globalThis.fetch(...args);
