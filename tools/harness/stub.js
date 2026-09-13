// The `window.__TAURI__` a page gets in the harness.
//
// Installed with addInitScript so it exists before any module runs - the pages
// destructure it at the top level, so a stub installed after navigation is a
// stub that arrives too late.
//
// Commands are whatever the scenario passes in. Anything not listed rejects
// rather than resolving to undefined: a page that quietly carries on with an
// undefined answer is the kind of break this harness exists to catch.

/** Runs inside the page. Takes only cloneable data, so handlers cross as
 *  source text and are rebuilt on the other side. */
function install({ sources, kinds, assetBase }) {
  const calls = [];
  const listeners = new Map();

  const handlers = {};
  for (const [name, source] of Object.entries(sources)) {
    handlers[name] =
      kinds[name] === "function"
        ? new Function(`return (${source});`)()
        : () => JSON.parse(source);
  }

  window.__harness = {
    calls,
    closed: 0,
    alerts: [],
    /** Deliver a Tauri event to the page, the way the backend would. */
    emit(name, value) {
      for (const cb of listeners.get(name) ?? []) cb({ payload: value });
    },
    /** The arguments the page passed the last time it called `name`. */
    lastCall(name) {
      return [...calls].reverse().find((c) => c.name === name) ?? null;
    },
  };

  window.alert = (message) => window.__harness.alerts.push(String(message));

  window.__TAURI__ = {
    core: {
      async invoke(name, args) {
        calls.push({ name, args });
        const handler = handlers[name];
        if (!handler) throw new Error(`harness: no stub for command "${name}"`);
        return await handler(args);
      },
      // Mirrors Tauri: the page never sees a filesystem path, only a URL it
      // can fetch. Keeping the basename means the served file can be found.
      convertFileSrc: (path) =>
        `${assetBase}/${String(path).replace(/^.*[\\/]/, "")}`,
    },
    window: {
      getCurrentWindow: () => ({
        close: () => window.__harness.closed++,
        hide: () => {},
        show: () => {},
        setFocus: () => {},
      }),
    },
    event: {
      listen(name, cb) {
        if (!listeners.has(name)) listeners.set(name, []);
        listeners.get(name).push(cb);
        return Promise.resolve(() => {});
      },
      emit: () => Promise.resolve(),
    },
  };
}

/** Turn a scenario into something `install` can be handed across the bridge. */
function payload({ commands = {}, assetBase = "" } = {}) {
  const sources = {};
  const kinds = {};
  for (const [name, value] of Object.entries(commands)) {
    kinds[name] = typeof value === "function" ? "function" : "value";
    sources[name] =
      typeof value === "function" ? value.toString() : JSON.stringify(value);
  }
  return { sources, kinds, assetBase };
}

module.exports = { install, payload };
