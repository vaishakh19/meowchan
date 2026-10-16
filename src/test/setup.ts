import '@testing-library/react';

/**
 * Vitest global setup.
 *
 * Diskwise is a desktop app, but the layout/formatting/entitlement logic is pure and is tested
 * in jsdom. Tauri-specific globals are stubbed here so tests can exercise the transport layer
 * without a real webview.
 */

if (!('matchMedia' in window) || typeof window.matchMedia !== 'function') {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

if (!('ResizeObserver' in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = ResizeObserverStub;
}

if (!('scrollTo' in window) || typeof window.scrollTo !== 'function') {
  Object.defineProperty(window, 'scrollTo', { writable: true, value: () => {} });
}
