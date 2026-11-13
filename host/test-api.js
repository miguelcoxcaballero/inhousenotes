// window.__IHN_TEST_API__ — loaded only on localhost with ?e2e=1 (TESTING §2). Assertions read state through
// `cmd`; actions go through the UI.
export default function install(host, { KIND, IMPORTS }) {
  const cmd = (name, args = null) => {
    const json = JSON.stringify(args) ?? 'null';
    host.enter(() => {
      const w = host.writer.begin(8 + (name.length + json.length) * 3).str(name).str(json);
      return host.ex.ihn_event(KIND.TEST, w.base, w.length);
    });
    const result = host.testResult();
    return result === null ? undefined : JSON.parse(result);
  };
  window.__IHN_TEST_API__ = Object.freeze({
    // Resolves when Rust reports that boot has finished.
    ready: () => host.ready,
    // `<module>.<command>` handled by that module's `test_cmd` (rust/crates/ihn-app/src/testapi.rs).
    cmd,
    state: () => cmd('app.state'),
    // The imports the running module really has, and the allowlist the build enforces (ARCHITECTURE §3.2).
    wasmImports: () => WebAssembly.Module.imports(host.module).map(entry => `${entry.module}.${entry.name}`),
    allowedImports: () => IMPORTS.map(name => `env.${name}`),
    wasmMemoryBytes: () => host.memory.buffer.byteLength
  });
}
