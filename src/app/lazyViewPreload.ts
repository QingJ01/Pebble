export type LazyViewImporter = () => Promise<unknown>;

type IdleWindow = Window & {
  requestIdleCallback?: (callback: IdleRequestCallback, options?: IdleRequestOptions) => number;
  cancelIdleCallback?: (handle: number) => void;
};

export function createLazyViewPreloader(importers: readonly LazyViewImporter[]) {
  let preloadPromise: Promise<PromiseSettledResult<unknown>[]> | null = null;

  return function preloadLazyViews() {
    if (!preloadPromise) {
      preloadPromise = Promise.allSettled(importers.map((importer) => importer()));
    }
    return preloadPromise;
  };
}

export function scheduleLazyViewPreload(
  preload: () => Promise<PromiseSettledResult<unknown>[]>,
  win: Window = window,
) {
  const idleWindow = win as IdleWindow;
  const run = () => {
    void preload();
  };

  if (idleWindow.requestIdleCallback) {
    const handle = idleWindow.requestIdleCallback(run, { timeout: 1200 });
    return () => idleWindow.cancelIdleCallback?.(handle);
  }

  const handle = win.setTimeout(run, 0);
  return () => win.clearTimeout(handle);
}
